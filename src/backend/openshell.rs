//! Opt-in CLI adapter for an existing NVIDIA OpenShell gateway.
use super::{
    BackendType, ExecOptions, ExecResult, RemoteSandboxContext, Sandbox, SandboxConfig,
    SandboxRuntimeMetadata,
};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use serde::Deserialize;
use std::{
    collections::HashMap,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    process::Command,
    time::{sleep, timeout},
};
use uuid::Uuid;

const GATEWAY: &str = "AGENTKERNEL_OPENSHELL_GATEWAY";
const WORKSPACE: &str = "AGENTKERNEL_OPENSHELL_WORKSPACE";
const OWNER_LABEL: &str = "agentkernel.dev/owner";

// NVIDIA OpenShell v0.1.9 rejects sandbox names longer than 19 bytes.
// The compact name is only a route, never an ownership check: keep the
// full UUID label and pinned provider ID for that.
fn provider_name(owner: Uuid) -> String {
    let hex = owner.simple().to_string();
    format!("ak-{}", &hex[..16])
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    gateway: String,
    workspace: String,
}

impl Target {
    fn new(gateway: String, workspace: String) -> Result<Self> {
        for value in [&gateway, &workspace] {
            ensure!(
                !value.starts_with('-')
                    && !value.is_empty()
                    && value.len() <= 63
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
                "Invalid OpenShell gateway or workspace name"
            );
        }
        Ok(Self { gateway, workspace })
    }

    fn from_env() -> Result<Self> {
        Self::new(
            std::env::var(GATEWAY).context(
                "Set AGENTKERNEL_OPENSHELL_GATEWAY to an existing OpenShell gateway name",
            )?,
            std::env::var(WORKSPACE).context(
                "Set AGENTKERNEL_OPENSHELL_WORKSPACE to a dedicated existing OpenShell workspace",
            )?,
        )
    }

    fn arguments(&self) -> [&str; 7] {
        [
            "--gateway",
            &self.gateway,
            "--workspace",
            &self.workspace,
            "--color",
            "never",
            "sandbox",
        ]
    }
}

pub fn target_configured() -> bool {
    Target::from_env().is_ok()
}

pub fn available() -> bool {
    target_configured() && super::command_available("openshell")
}

#[derive(Debug, Deserialize)]
struct Listing {
    sandboxes: Vec<Info>,
    // OpenShell v0.1.9 always emits the continuation string.
    next_page_token: String,
}

#[derive(Debug, Deserialize)]
struct Info {
    id: String,
    name: String,
    workspace: String,
    phase: String,
    labels: HashMap<String, String>,
}

pub struct OpenShellSandbox {
    name: String,
    target: Target,
    remote_name: String,
    owner: String,
    id: Option<String>,
    running: bool,
    cli: String,
}

impl OpenShellSandbox {
    pub fn new(name: &str, state: RemoteSandboxContext) -> Result<Self> {
        let target = Target::from_env()?;
        let meta = state.remote_metadata;
        let (owner, remote_name) = if state.remote_id.is_some()
            || meta.contains_key("openshell_owner")
        {
            ensure!(
                meta.get("openshell_gateway") == Some(&target.gateway)
                    && state.remote_namespace.as_ref() == Some(&target.workspace),
                "OpenShell gateway/workspace differs from this sandbox's saved target"
            );
            let owner = meta
                .get("openshell_owner")
                .context("Missing OpenShell ownership binding")?
                .clone();
            let nonce = Uuid::parse_str(&owner).context("Invalid OpenShell ownership binding")?;
            (owner, provider_name(nonce))
        } else {
            let nonce = Uuid::new_v4();
            (nonce.to_string(), provider_name(nonce))
        };
        Ok(Self {
            name: name.to_string(),
            target,
            remote_name,
            owner,
            id: state.remote_id,
            running: meta
                .get("last_known_status")
                .is_some_and(|status| status == "running"),
            cli: "openshell".into(),
        })
    }

    async fn invoke(&self, args: &[&str], wait_secs: u64) -> Result<Output> {
        let mut cli = Command::new(&self.cli);
        cli.args(self.target.arguments())
            .args(args)
            // Never allow an ambient endpoint or insecure TLS override to
            // take precedence over the selected existing gateway.
            .env_remove("OPENSHELL_GATEWAY_ENDPOINT")
            .env_remove("OPENSHELL_GATEWAY_INSECURE")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        timeout(Duration::from_secs(wait_secs), cli.output())
            .await
            .with_context(|| format!("OpenShell {} timed out", args[0]))?
            .with_context(|| format!("Could not run OpenShell {}", args[0]))
    }

    async fn checked(&self, args: &[&str], wait_secs: u64) -> Result<Output> {
        let output = self.invoke(args, wait_secs).await?;
        // Do not surface arbitrary gateway/CLI diagnostics (which can
        // contain operator configuration or credentials) in server errors.
        ensure!(
            output.status.success(),
            "OpenShell sandbox {} failed ({})",
            args[0],
            output.status
        );
        Ok(output)
    }

    fn verify(&self, info: &Info) -> Result<()> {
        ensure!(
            !info.id.is_empty()
                && self.id.as_ref().is_none_or(|id| id == &info.id)
                && info.name == self.remote_name
                && info.workspace == self.target.workspace
                && info.labels.get(OWNER_LABEL) == Some(&self.owner),
            "OpenShell sandbox identity did not match its AgentKernel ownership binding"
        );
        Ok(())
    }

    async fn find(&self) -> Result<Option<Info>> {
        let selector = format!("{OWNER_LABEL}={}", self.owner);
        let output = self
            .checked(
                &[
                    "list",
                    "--selector",
                    &selector,
                    "--page-size",
                    "2",
                    "--output",
                    "json",
                ],
                15,
            )
            .await?;
        let mut page: Listing =
            serde_json::from_slice(&output.stdout).context("Invalid OpenShell list output")?;
        ensure!(
            page.sandboxes.len() <= 1 && page.next_page_token.is_empty(),
            "OpenShell ownership lookup was not unique"
        );
        if let Some(info) = page.sandboxes.pop() {
            self.verify(&info)?;
            Ok(Some(info))
        } else {
            Ok(None)
        }
    }

    async fn ready(&self) -> Result<()> {
        let info = self
            .find()
            .await?
            .context("OpenShell sandbox is no longer available")?;
        ensure!(info.phase == "Ready", "OpenShell sandbox is not Ready");
        Ok(())
    }

    async fn delete_owned(&mut self) -> Result<()> {
        if self.find().await?.is_some() {
            self.checked(&["delete", &self.remote_name], 30).await?;
            // Delete can report acceptance before cleanup completes.
            for _ in 0..10 {
                if self.find().await?.is_none() {
                    self.running = false;
                    return Ok(());
                }
                sleep(Duration::from_millis(300)).await;
            }
            bail!("OpenShell deletion was accepted but cleanup has not finished");
        }
        self.running = false;
        Ok(())
    }
}

fn validate_config(config: &SandboxConfig) -> Result<()> {
    ensure!(
        !config.mount_cwd
            && !config.mount_home
            && config.work_dir.is_none()
            && config.container_work_dir.is_none()
            && config.volumes.is_empty()
            && config.files.is_empty()
            && config.ssh.is_none()
            && config.ports.is_empty()
            && config.managed_network.is_none()
            && config.env.is_empty(),
        "OpenShell does not support AgentKernel mounts, files, environment injection, SSH, or ports"
    );
    ensure!(
        config.network && !config.read_only,
        "OpenShell cannot translate AgentKernel offline or read-only root requests"
    );
    let image = config.image.as_str();
    // OpenShell also accepts a host-local rootfs archive in --from. This
    // adapter accepts OCI references only and must never upload a host file.
    ensure!(
        image
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && image
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-:@".contains(&b))
            && !image
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            && ![".tar", ".tar.gz", ".tgz"]
                .iter()
                .any(|suffix| image.to_ascii_lowercase().ends_with(suffix)),
        "OpenShell requires an OCI image reference, not a host path or archive"
    );
    Ok(())
}

#[async_trait]
impl Sandbox for OpenShellSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        validate_config(config)?;
        if let Some(info) = self.find().await? {
            // Recover the durable ownership record if the previous process
            // crashed after creation but before it saved the provider ID.
            self.id = Some(info.id);
            if info.phase != "Ready" {
                ensure!(
                    info.phase == "Stopped",
                    "OpenShell sandbox must be Stopped before restarting"
                );
                self.checked(&["start", &self.remote_name], 330).await?;
            }
            self.ready().await?;
        } else {
            ensure!(
                self.id.is_none(),
                "Previously created OpenShell sandbox is no longer available"
            );
            let owner = format!("{OWNER_LABEL}={}", self.owner);
            let cpu = config.vcpus.to_string();
            let memory = format!("{}Mi", config.memory_mb);
            let create = self
                .checked(
                    &[
                        "create",
                        "--name",
                        &self.remote_name,
                        "--label",
                        &owner,
                        "--from",
                        &config.image,
                        "--cpu",
                        &cpu,
                        "--memory",
                        &memory,
                        "--no-auto-providers",
                        "--detach",
                        "--output",
                        "json",
                        "--timeout",
                        "300",
                        "--",
                        "/bin/sh",
                        "-c",
                        "while :; do sleep 3600; done",
                    ],
                    330,
                )
                .await;
            let verified = match create {
                Ok(output) => serde_json::from_slice::<Info>(&output.stdout)
                    .context("Invalid OpenShell create output")
                    .and_then(|info| {
                        self.verify(&info)?;
                        ensure!(
                            info.phase == "Ready",
                            "OpenShell sandbox did not become Ready"
                        );
                        Ok(info)
                    }),
                Err(error) => Err(error),
            };
            match verified {
                Ok(info) => self.id = Some(info.id),
                Err(error) => {
                    // Only delete an object matching the unpredictable owner label.
                    if let Err(cleanup) = self.delete_owned().await {
                        return Err(error).context(format!(
                            "Could not reconcile OpenShell sandbox '{}' after create failure: {cleanup}", self.remote_name
                        ));
                    }
                    return Err(error);
                }
            }
        }
        self.running = true;
        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_options(cmd, &ExecOptions::default()).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        ensure!(
            env.is_empty(),
            "OpenShell does not support per-command environment injection"
        );
        self.exec(cmd).await
    }

    async fn exec_with_options(&mut self, cmd: &[&str], opts: &ExecOptions) -> Result<ExecResult> {
        ensure!(!cmd.is_empty(), "OpenShell requires a command");
        ensure!(
            opts.env.is_empty() && opts.user.is_none(),
            "OpenShell does not support per-command environment or user overrides"
        );
        ensure!(self.id.is_some(), "No OpenShell sandbox has been started");
        self.ready().await?;
        let mut args = vec![
            "exec",
            "--name",
            &self.remote_name,
            "--no-tty",
            "--no-login-shell",
            "--timeout",
            "300",
        ];
        if let Some(workdir) = opts.workdir.as_deref() {
            args.extend(["--workdir", workdir]);
        }
        args.push("--");
        args.extend_from_slice(cmd);
        let output = self.invoke(&args, 315).await?;
        Ok(ExecResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn stop(&mut self) -> Result<()> {
        if self.find().await?.is_some() {
            self.checked(&["stop", &self.remote_name], 330).await?;
        }
        self.running = false;
        Ok(())
    }

    async fn remove(&mut self) -> Result<()> {
        self.delete_owned().await
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn backend_type(&self) -> BackendType {
        BackendType::OpenShell
    }
    fn is_running(&self) -> bool {
        self.running
    }

    async fn write_file_unchecked(&mut self, _path: &str, _content: &[u8]) -> Result<()> {
        bail!("Files are not supported by the experimental OpenShell backend")
    }
    async fn read_file_unchecked(&mut self, _path: &str) -> Result<Vec<u8>> {
        bail!("Files are not supported by the experimental OpenShell backend")
    }
    async fn remove_file_unchecked(&mut self, _path: &str) -> Result<()> {
        bail!("Files are not supported by the experimental OpenShell backend")
    }
    async fn mkdir_unchecked(&mut self, _path: &str, _recursive: bool) -> Result<()> {
        bail!("Files are not supported by the experimental OpenShell backend")
    }

    fn runtime_metadata(&self) -> Option<SandboxRuntimeMetadata> {
        Some(SandboxRuntimeMetadata {
            remote_id: self.id.clone(),
            remote_namespace: Some(self.target.workspace.clone()),
            remote_metadata: HashMap::from([
                ("openshell_gateway".into(), self.target.gateway.clone()),
                ("openshell_owner".into(), self.owner.clone()),
                (
                    "last_known_status".into(),
                    if self.running { "running" } else { "stopped" }.into(),
                ),
            ]),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_name_fits_the_vendor_dns_label_limit() {
        let owner = Uuid::parse_str("12345678-abcd-42ef-aaaa-123456789abc").unwrap();
        let name = provider_name(owner);
        assert_eq!(name, "ak-12345678abcd42ef");
        assert_eq!(name.len(), 19);
        assert!(
            name.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        );
    }

    #[test]
    fn validates_target_and_rejects_unsupported_constraints() {
        assert!(Target::new("test-gateway".into(), "sandbox-workspace".into()).is_ok());
        for value in ["", "--insecure", "two words", "name/other"] {
            assert!(Target::new(value.into(), "default".into()).is_err());
        }
        assert!(validate_config(&SandboxConfig::default()).is_ok());
        for image in [
            "./secret.tar",
            "rootfs.tar",
            "/private/path",
            "oci://image",
            "--flag",
        ] {
            assert!(
                validate_config(&SandboxConfig::with_image(image)).is_err(),
                "{image}"
            );
        }
        let config = SandboxConfig::default().with_network(false);
        assert!(validate_config(&config).is_err());
        let config = SandboxConfig::default().with_env(vec![("DUMMY".into(), "value".into())]);
        assert!(validate_config(&config).is_err());
        let config = SandboxConfig {
            read_only: true,
            ..Default::default()
        };
        assert!(validate_config(&config).is_err());
        let config = SandboxConfig::default().with_mount_cwd(true, None);
        assert!(validate_config(&config).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_contract_persists_ownership_and_never_executes_or_deletes_replacements() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let cli_path = dir.path().join("openshell");
        let script = r#"#!/bin/sh
set -eu
cd "$(dirname "$0")"
[ "$1 $2 $3 $4 $5 $6 $7" = "--gateway test-gateway --workspace test-workspace --color never sandbox" ]
shift 7
op="$1"
shift
echo "$op" >> calls
owner=12345678-abcd-42ef-aaaa-123456789abc
target=ak-12345678abcd42ef
info() {
    printf '{"id":"%s","name":"%s","workspace":"test-workspace","phase":"%s","labels":{"agentkernel.dev/owner":"%s"}}' "$(cat id)" "$target" "$(cat phase)" "$owner"
}
case "$op" in
  list)
    [ "$*" = "--selector agentkernel.dev/owner=$owner --page-size 2 --output json" ]
    printf '{"sandboxes":['
    if [ -f id ]; then info; fi
    printf '],"next_page_token":""}'
    ;;
  create)
    [ "$*" = "--name $target --label agentkernel.dev/owner=$owner --from alpine:3.24 --cpu 1 --memory 512Mi --no-auto-providers --detach --output json --timeout 300 -- /bin/sh -c while :; do sleep 3600; done" ]
    printf provider-id > id
    printf Ready > phase
    info
    ;;
  exec)
    [ "$1 $2 $3 $4 $5 $6 $7 $8 $9" = "--name $target --no-tty --no-login-shell --timeout 300 --workdir /sandbox --" ]
    shift 9
    [ "$1" = "printf" ] && [ "$2" = "literal ; no host shell" ]
    printf 'sandbox stdout'
    printf 'sandbox stderr' >&2
    exit 23
    ;;
  stop)
    [ "$1" = "$target" ]
    printf Stopped > phase
    ;;
  start)
    [ "$1" = "$target" ]
    printf Ready > phase
    ;;
  delete)
    [ "$1" = "$target" ]
    rm id phase
    ;;
  *) exit 99;;
esac
"#;
        fs::write(&cli_path, script).unwrap();
        fs::set_permissions(&cli_path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut backend = OpenShellSandbox {
            name: "test".into(),
            target: Target::new("test-gateway".into(), "test-workspace".into()).unwrap(),
            remote_name: provider_name(
                Uuid::parse_str("12345678-abcd-42ef-aaaa-123456789abc").unwrap(),
            ),
            owner: "12345678-abcd-42ef-aaaa-123456789abc".into(),
            id: None,
            running: false,
            cli: cli_path.to_string_lossy().into_owned(),
        };

        let binding = backend.runtime_metadata().unwrap();
        assert!(
            binding.remote_id.is_none(),
            "record ownership before create"
        );
        assert_eq!(binding.remote_metadata["openshell_owner"], backend.owner);
        backend.start(&SandboxConfig::default()).await.unwrap();
        assert_eq!(
            backend.runtime_metadata().unwrap().remote_id.as_deref(),
            Some("provider-id")
        );
        let result = backend
            .exec_with_options(
                &["printf", "literal ; no host shell"],
                &ExecOptions {
                    workdir: Some("/sandbox".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            (
                result.exit_code,
                result.stdout.as_str(),
                result.stderr.as_str()
            ),
            (23, "sandbox stdout", "sandbox stderr")
        );
        backend.stop().await.unwrap();
        assert!(!backend.is_running());
        backend.start(&SandboxConfig::default()).await.unwrap();
        assert!(backend.is_running());
        fs::write(dir.path().join("id"), "different-provider-id").unwrap();
        assert!(backend.exec(&["must-not-run"]).await.is_err());
        assert!(backend.remove().await.is_err());
        assert!(dir.path().join("id").exists());
        fs::write(dir.path().join("id"), "provider-id").unwrap();
        backend.remove().await.unwrap();
        assert!(!dir.path().join("id").exists());
        let calls = fs::read_to_string(dir.path().join("calls")).unwrap();
        assert_eq!(calls.lines().filter(|call| *call == "create").count(), 1);
        assert_eq!(calls.lines().filter(|call| *call == "exec").count(), 1);
        assert_eq!(calls.lines().filter(|call| *call == "delete").count(), 1);
    }
}
