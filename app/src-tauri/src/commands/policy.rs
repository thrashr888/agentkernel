use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use tauri::State;

use crate::state::AppState;
use crate::types::{
    PolicyActivationRequest, PolicyActivationResult, PolicyAuditEntry, PolicyCheckResult,
    PolicyEditorMaterial, PolicyReloadResult, PolicyStatus,
};

const LOCAL_POLICY_FILE_NAME: &str = "policy.cedar";

/// Parse Cedar material before replacing either local file. The sidecar still
/// performs its normal startup validation after restart, but malformed editor
/// input is rejected without a write or process restart.
fn validate_cedar(policy: &str) -> Result<(), String> {
    let trimmed = policy.trim();
    if trimmed.is_empty() {
        return Err("Cedar policy must not be empty".to_string());
    }
    let _: cedar_policy::PolicySet = trimmed
        .parse()
        .map_err(|error| format!("Invalid Cedar policy: {error}"))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct FileSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

/// Parse both activation inputs before touching either on-disk file. The
/// desktop owns the policy file name so a config cannot redirect activation
/// to an arbitrary path outside its app-managed config directory.
fn prepare_activation(request: &PolicyActivationRequest) -> Result<String, String> {
    let mut config: toml::Value =
        toml::from_str(&request.config).map_err(|e| format!("Invalid AgentKernel TOML: {e}"))?;

    let table = config
        .as_table_mut()
        .ok_or_else(|| "AgentKernel config must be a TOML table".to_string())?;
    let enterprise = table
        .entry("enterprise")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[enterprise] must be a TOML table".to_string())?;
    if let Some(enabled) = enterprise.get("enabled") {
        if !enabled.is_bool() {
            return Err("[enterprise].enabled must be a boolean".to_string());
        }
    }
    enterprise.insert(
        "policy_file".to_string(),
        toml::Value::String(LOCAL_POLICY_FILE_NAME.to_string()),
    );

    validate_cedar(&request.policy)?;

    toml::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize AgentKernel TOML: {e}"))
}

fn snapshot(path: &Path) -> Result<FileSnapshot, String> {
    let contents = match fs::read(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Failed to read {}: {error}", path.display())),
    };
    Ok(FileSnapshot {
        path: path.to_path_buf(),
        contents,
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    path.with_file_name(format!(".{name}.tmp-{}-{nanos}", std::process::id()))
}

/// Write a file by fsyncing a sibling temporary file and renaming it into
/// place. A policy activation never exposes a partially-written TOML or
/// Cedar file to a concurrently starting sidecar.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    let temp = temporary_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(replacement_mode(path));
        let mut file = options
            .open(&temp)
            .map_err(|e| format!("Failed to create {}: {e}", temp.display()))?;
        #[cfg(unix)]
        file.set_permissions(std::fs::Permissions::from_mode(replacement_mode(path)))
            .map_err(|e| format!("Failed to protect {}: {e}", temp.display()))?;
        file.write_all(contents)
            .map_err(|e| format!("Failed to write {}: {e}", temp.display()))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync {}: {e}", temp.display()))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| format!("Failed to replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn replacement_mode(path: &Path) -> u32 {
    let existing = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o7777);
    // Config and policy files can contain API keys and tenant authorization
    // rules. New files are private, and an existing private mode (including
    // read-only modes) is preserved. A previously broad mode is hardened.
    existing.filter(|mode| mode & 0o077 == 0).unwrap_or(0o600)
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.bak",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
    ))
}

fn write_backup(snapshot: &FileSnapshot) -> Result<Option<PathBuf>, String> {
    let Some(contents) = snapshot.contents.as_ref() else {
        return Ok(None);
    };
    let backup = backup_path(&snapshot.path);
    atomic_write(&backup, contents)?;
    Ok(Some(backup))
}

fn restore(snapshot: &FileSnapshot) -> Result<(), String> {
    match snapshot.contents.as_ref() {
        Some(contents) => atomic_write(&snapshot.path, contents),
        None => match fs::remove_file(&snapshot.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "Failed to remove {}: {error}",
                snapshot.path.display()
            )),
        },
    }
}

fn rollback_files(config: &FileSnapshot, policy: &FileSnapshot) -> Result<(), String> {
    // Restore both files even if the first restoration fails, then report all
    // failures to the caller so a failed activation is never mistaken for a
    // successful rollback.
    let config_error = restore(config).err();
    let policy_error = restore(policy).err();
    match (config_error, policy_error) {
        (None, None) => Ok(()),
        (Some(config), None) => Err(config),
        (None, Some(policy)) => Err(policy),
        (Some(config), Some(policy)) => Err(format!("{config}; {policy}")),
    }
}

fn policy_path_for_config(config_path: &Path) -> Result<PathBuf, String> {
    config_path
        .parent()
        .map(|parent| parent.join(LOCAL_POLICY_FILE_NAME))
        .ok_or_else(|| format!("{} has no parent directory", config_path.display()))
}

fn config_enables_policy(config: &str) -> bool {
    toml::from_str::<toml::Value>(config)
        .ok()
        .and_then(|value| {
            value
                .get("enterprise")
                .and_then(|enterprise| enterprise.get("enabled"))
                .and_then(toml::Value::as_bool)
        })
        .unwrap_or(false)
}

fn rollback_message(
    operation: &str,
    file_error: Option<String>,
    restart_error: Option<String>,
) -> String {
    let mut details = Vec::new();
    if let Some(error) = file_error {
        details.push(format!("file rollback failed: {error}"));
    }
    if let Some(error) = restart_error {
        details.push(format!("server rollback restart failed: {error}"));
    }
    if details.is_empty() {
        format!("{operation}; rollback completed")
    } else {
        format!("{operation}; {}", details.join("; "))
    }
}

#[derive(Debug, Clone, Copy)]
enum PolicyVerification {
    Activated { enabled: bool },
    Healthy,
}

/// Run the file mutation and owned-sidecar lifecycle as one testable
/// transaction. The callbacks are deliberately injected so unit tests can
/// exercise restart, verification, and rollback without starting a process or
/// contacting a real server.
#[allow(clippy::too_many_arguments)]
async fn run_activation_transaction<Restart, RestartFuture, Verify, VerifyFuture>(
    config_path: &Path,
    policy_path: &Path,
    old_config: FileSnapshot,
    old_policy: FileSnapshot,
    prepared_config: &str,
    policy_contents: &str,
    expected_enabled: bool,
    mut restart: Restart,
    mut verify: Verify,
) -> Result<PolicyActivationResult, String>
where
    Restart: FnMut() -> RestartFuture,
    RestartFuture: Future<Output = Result<String, String>>,
    Verify: FnMut(PolicyVerification) -> VerifyFuture,
    VerifyFuture: Future<Output = Result<PolicyStatus, String>>,
{
    let config_backup = write_backup(&old_config)?;
    let policy_backup = match write_backup(&old_policy) {
        Ok(path) => path,
        Err(error) => return Err(format!("Could not create policy backup: {error}")),
    };

    if let Err(error) = atomic_write(config_path, prepared_config.as_bytes()) {
        return Err(format!("Policy activation did not change files: {error}"));
    }
    if let Err(error) = atomic_write(policy_path, policy_contents.as_bytes()) {
        let file_error = rollback_files(&old_config, &old_policy).err();
        return Err(rollback_message(
            &format!("Policy activation failed while writing policy: {error}"),
            file_error,
            None,
        ));
    }

    let activation_result = async {
        restart().await?;
        verify(PolicyVerification::Activated {
            enabled: expected_enabled,
        })
        .await
    }
    .await;

    match activation_result {
        Ok(status) => Ok(PolicyActivationResult {
            status,
            config_path: config_path.display().to_string(),
            policy_path: policy_path.display().to_string(),
            config_backup: config_backup.map(|path| path.display().to_string()),
            policy_backup: policy_backup.map(|path| path.display().to_string()),
            rolled_back: false,
        }),
        Err(error) => {
            let file_error = rollback_files(&old_config, &old_policy).err();
            let restart_error = match restart().await {
                Err(error) => Some(error),
                Ok(_) => verify(PolicyVerification::Healthy).await.err(),
            };
            Err(rollback_message(
                &format!("Policy activation failed: {error}"),
                file_error,
                restart_error,
            ))
        }
    }
}

fn active_local_config(state: &AppState) -> Result<PathBuf, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    let entry = settings
        .active()
        .ok_or_else(|| "Policy activation requires an active server".to_string())?;
    if !crate::commands::server::owns_local_server(entry) {
        return Err(
            "Remote and separately managed servers are read-only. Ask the server administrator to update Cedar policy.".to_string(),
        );
    }
    crate::commands::server::config_path_for_server(entry)
}

async fn wait_for_activation_status(
    client: &crate::api_client::ApiClient,
    enabled: bool,
) -> Result<PolicyStatus, String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut last_error;
    loop {
        match client.get_policy_status().await {
            Ok(status) => {
                let valid = if enabled {
                    status.active && status.enforcing && status.healthy
                } else {
                    !status.configured && !status.active && status.healthy
                };
                if valid {
                    return Ok(status);
                }
                last_error = Some(format!(
                    "server reported compiled={}, configured={}, active={}, enforcing={}, healthy={}, source={:?}, init_error={:?}",
                    status.compiled,
                    status.configured,
                    status.active,
                    status.enforcing,
                    status.healthy,
                    status.source,
                    status.initialization_error,
                ));
            }
            Err(error) => last_error = Some(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Activated server did not become healthy and enforcing: {}",
                last_error.unwrap_or_else(|| "no status response".to_string())
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_healthy_status(
    client: &crate::api_client::ApiClient,
) -> Result<PolicyStatus, String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut last_error;
    loop {
        match client.get_policy_status().await {
            Ok(status) if status.healthy => return Ok(status),
            Ok(status) => {
                last_error = Some(format!(
                    "server reported healthy=false, active={}, enforcing={}, initialization_error={:?}",
                    status.active, status.enforcing, status.initialization_error
                ));
            }
            Err(error) => last_error = Some(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Rolled-back server did not become healthy: {}",
                last_error.unwrap_or_else(|| "no status response".to_string())
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_policy_status(state: State<'_, AppState>) -> Result<PolicyStatus, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.get_policy_status().await.map_err(|e| e.to_string())
}

/// Read the app-owned local material for the desktop editor. Remote and
/// separately managed servers intentionally fail this command so the UI
/// cannot accidentally imply that it can mutate their configuration.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_local_policy_material(
    state: State<'_, AppState>,
) -> Result<PolicyEditorMaterial, String> {
    let config_path = active_local_config(&state)?;
    let policy_path = policy_path_for_config(&config_path)?;
    let config = read_editor_file(
        &config_path,
        "[enterprise]\nenabled = true\noffline_mode = \"default_policy\"\n",
    )?;
    let policy = read_editor_file(
        &policy_path,
        "// Replace this starter policy with your Cedar rules.\npermit(principal, action, resource);\n",
    )?;
    Ok(PolicyEditorMaterial {
        config,
        policy,
        config_path: config_path.display().to_string(),
        policy_path: policy_path.display().to_string(),
    })
}

fn read_editor_file(path: &Path, default: &str) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(default.to_string()),
        Err(error) => Err(format!("Failed to read {}: {error}", path.display())),
    }
}

/// Atomically activate local policy material and restart only the sidecar
/// process owned by this desktop instance. Both files are restored and the
/// old configuration is restarted if validation, startup, or health checks
/// fail. Remote and separately managed servers are rejected before any write.
#[tauri::command(rename_all = "snake_case")]
pub async fn activate_local_policy(
    state: State<'_, AppState>,
    server_process: State<'_, crate::commands::server::ServerProcess>,
    request: PolicyActivationRequest,
) -> Result<PolicyActivationResult, String> {
    let prepared_config = prepare_activation(&request)?;
    let config_path = active_local_config(&state)?;
    let policy_path = policy_path_for_config(&config_path)?;
    let old_config = snapshot(&config_path)?;
    let old_policy = snapshot(&policy_path)?;
    let expected_enabled = config_enables_policy(&prepared_config);
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    run_activation_transaction(
        &config_path,
        &policy_path,
        old_config,
        old_policy,
        &prepared_config,
        &request.policy,
        expected_enabled,
        || async { crate::commands::server::restart_owned_server(&server_process, &state) },
        |verification| {
            let client = client.clone();
            async move {
                match verification {
                    PolicyVerification::Activated { enabled } => {
                        wait_for_activation_status(&client, enabled).await
                    }
                    PolicyVerification::Healthy => wait_for_healthy_status(&client).await,
                }
            }
        },
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_policy(
    state: State<'_, AppState>,
    action: String,
    sandbox: String,
) -> Result<PolicyCheckResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .check_policy(&action, &sandbox)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn reload_policy(state: State<'_, AppState>) -> Result<PolicyReloadResult, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client.reload_policy().await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_policy_audit(
    state: State<'_, AppState>,
    last: Option<u32>,
) -> Result<Vec<PolicyAuditEntry>, String> {
    let client = state.client.lock().map_err(|e| e.to_string())?.clone();
    client
        .get_policy_audit(last)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agentkernel-policy-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    const VALID_CEDAR: &str = "permit(principal, action, resource);";

    #[test]
    fn disabled_then_enabled_configs_are_validated_without_side_effects() {
        let disabled = PolicyActivationRequest {
            config: "[enterprise]\nenabled = false\n".to_string(),
            policy: VALID_CEDAR.to_string(),
        };
        let disabled_config = prepare_activation(&disabled).unwrap();
        assert!(disabled_config.contains("enabled = false"));

        let enabled = PolicyActivationRequest {
            config: "[enterprise]\nenabled = true\n".to_string(),
            policy: VALID_CEDAR.to_string(),
        };
        let enabled_config = prepare_activation(&enabled).unwrap();
        assert!(enabled_config.contains("enabled = true"));
        assert!(enabled_config.contains("policy_file = \"policy.cedar\""));
    }

    #[test]
    fn invalid_material_leaves_both_existing_files_unchanged() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        let policy_path = dir.join(LOCAL_POLICY_FILE_NAME);
        fs::write(&config_path, "old config").unwrap();
        fs::write(&policy_path, "old policy").unwrap();
        let request = PolicyActivationRequest {
            config: "[enterprise]\nenabled = true\n".to_string(),
            policy: "not Cedar".to_string(),
        };
        assert!(prepare_activation(&request).is_err());
        assert_eq!(fs::read(&config_path).unwrap(), b"old config");
        assert_eq!(fs::read(&policy_path).unwrap(), b"old policy");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remote_server_activation_is_read_only() {
        let settings = crate::state::Settings {
            active_server: Some("Remote".to_string()),
            servers: vec![crate::state::ServerEntry {
                name: "Remote".to_string(),
                url: "https://policy.example.test".to_string(),
                api_key: None,
                managed: Some(false),
                ssh_tunnel: None,
                config_path: None,
            }],
            theme: "system".to_string(),
            poll_interval_ms: 3000,
            api_url: "https://policy.example.test".to_string(),
            api_key: None,
        };
        let state = AppState {
            settings: std::sync::Mutex::new(settings),
            client: std::sync::Mutex::new(crate::api_client::ApiClient::new(
                "https://policy.example.test",
                None,
            )),
        };
        let error = active_local_config(&state).unwrap_err();
        assert!(error.contains("read-only"));
    }

    #[test]
    fn file_transaction_restores_previous_pair_after_failure() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        let policy_path = dir.join(LOCAL_POLICY_FILE_NAME);
        fs::write(&config_path, "old config").unwrap();
        fs::write(&policy_path, "old policy").unwrap();
        let config = snapshot(&config_path).unwrap();
        let policy = snapshot(&policy_path).unwrap();
        atomic_write(&config_path, b"new config").unwrap();
        atomic_write(&policy_path, b"new policy").unwrap();
        rollback_files(&config, &policy).unwrap();
        assert_eq!(fs::read(&config_path).unwrap(), b"old config");
        assert_eq!(fs::read(&policy_path).unwrap(), b"old policy");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn editor_read_errors_are_not_masked_as_starter_content() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        fs::create_dir(&config_path).unwrap();
        let error = read_editor_file(&config_path, "starter").unwrap_err();
        assert!(error.contains("Failed to read"));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn activation_files_and_backups_are_private() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        fs::write(&config_path, "old config").unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        let snapshot = snapshot(&config_path).unwrap();
        let backup = write_backup(&snapshot).unwrap().unwrap();
        atomic_write(&config_path, b"new config").unwrap();
        assert_eq!(
            fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o400)).unwrap();
        atomic_write(&config_path, b"private config").unwrap();
        assert_eq!(
            fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn test_status(active: bool, enforcing: bool, healthy: bool) -> PolicyStatus {
        PolicyStatus {
            enabled: true,
            compiled: true,
            configured: active,
            active,
            enforcing,
            healthy,
            version: 1,
            org_id: None,
            offline_mode: Some("default_policy".to_string()),
            policy_server: None,
            source: Some("local".to_string()),
            policy_source: Some("local".to_string()),
            config_path: None,
            initialization_error: None,
            init_error: None,
            fail_closed: true,
            meaningful: true,
            admin_guidance: None,
        }
    }

    #[tokio::test]
    async fn activation_transaction_writes_pair_restarts_once_and_returns_enforced_status() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        let policy_path = dir.join(LOCAL_POLICY_FILE_NAME);
        fs::write(&config_path, "[enterprise]\nenabled = false\n").unwrap();
        fs::write(&policy_path, "permit(principal, action, resource);").unwrap();
        let old_config = snapshot(&config_path).unwrap();
        let old_policy = snapshot(&policy_path).unwrap();
        let mut restart_count = 0;
        let mut verification_count = 0;

        let result = run_activation_transaction(
            &config_path,
            &policy_path,
            old_config,
            old_policy,
            "[enterprise]\nenabled = true\npolicy_file = \"policy.cedar\"\n",
            "permit(principal, action, resource);",
            true,
            || {
                restart_count += 1;
                async { Ok("started".to_string()) }
            },
            |verification| {
                verification_count += 1;
                assert!(matches!(
                    verification,
                    PolicyVerification::Activated { enabled: true }
                ));
                async { Ok(test_status(true, true, true)) }
            },
        )
        .await
        .unwrap();

        assert_eq!(restart_count, 1);
        assert_eq!(verification_count, 1);
        assert!(!result.rolled_back);
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "[enterprise]\nenabled = true\npolicy_file = \"policy.cedar\"\n"
        );
        assert_eq!(
            fs::read_to_string(&policy_path).unwrap(),
            "permit(principal, action, resource);"
        );
        assert!(result.config_backup.is_some());
        assert!(result.policy_backup.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn verification_failure_restores_pair_restarts_old_config_and_checks_health() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        let policy_path = dir.join(LOCAL_POLICY_FILE_NAME);
        let old_config_text = "[enterprise]\nenabled = false\n";
        let old_policy_text = "permit(principal, action, resource);";
        fs::write(&config_path, old_config_text).unwrap();
        fs::write(&policy_path, old_policy_text).unwrap();
        let old_config = snapshot(&config_path).unwrap();
        let old_policy = snapshot(&policy_path).unwrap();
        let mut restart_count = 0;
        let mut verification_kinds = Vec::new();

        let error = run_activation_transaction(
            &config_path,
            &policy_path,
            old_config,
            old_policy,
            "[enterprise]\nenabled = true\npolicy_file = \"policy.cedar\"\n",
            "permit(principal, action, resource);",
            true,
            || {
                restart_count += 1;
                async { Ok("started".to_string()) }
            },
            |verification| {
                verification_kinds.push(verification);
                async move {
                    match verification {
                        PolicyVerification::Activated { .. } => {
                            Err("sidecar initialization failed".to_string())
                        }
                        PolicyVerification::Healthy => Ok(test_status(false, false, true)),
                    }
                }
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("sidecar initialization failed"));
        assert_eq!(restart_count, 2);
        assert_eq!(verification_kinds.len(), 2);
        assert!(matches!(verification_kinds[1], PolicyVerification::Healthy));
        assert_eq!(fs::read_to_string(&config_path).unwrap(), old_config_text);
        assert_eq!(fs::read_to_string(&policy_path).unwrap(), old_policy_text);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn rollback_restart_failure_is_reported_with_activation_error() {
        let dir = temp_dir();
        let config_path = dir.join("agentkernel.toml");
        let policy_path = dir.join(LOCAL_POLICY_FILE_NAME);
        fs::write(&config_path, "[enterprise]\nenabled = false\n").unwrap();
        fs::write(&policy_path, "permit(principal, action, resource);").unwrap();
        let old_config = snapshot(&config_path).unwrap();
        let old_policy = snapshot(&policy_path).unwrap();
        let mut restart_count = 0;

        let error = run_activation_transaction(
            &config_path,
            &policy_path,
            old_config,
            old_policy,
            "[enterprise]\nenabled = true\npolicy_file = \"policy.cedar\"\n",
            "permit(principal, action, resource);",
            true,
            || {
                restart_count += 1;
                let result = if restart_count == 2 {
                    Err("old sidecar could not restart".to_string())
                } else {
                    Ok("started".to_string())
                };
                async move { result }
            },
            |verification| async move {
                match verification {
                    PolicyVerification::Activated { .. } => {
                        Err("new policy failed initialization".to_string())
                    }
                    PolicyVerification::Healthy => Ok(test_status(false, false, true)),
                }
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("new policy failed initialization"));
        assert!(error.contains("server rollback restart failed: old sidecar could not restart"));
        assert_eq!(restart_count, 2);
        let _ = fs::remove_dir_all(dir);
    }
}
