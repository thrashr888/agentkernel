use super::{
    BackendType, ExecOptions, ExecResult, PortMapping, RemoteSandboxContext, ResolvedEndpoint,
    Sandbox, SandboxConfig, SandboxRuntimeMetadata,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::config::{Config, RemoteProviderConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteProvider {
    Daytona,
    Runloop,
    E2B,
    AgentComputer,
}

impl RemoteProvider {
    pub fn backend_type(self) -> BackendType {
        match self {
            Self::Daytona => BackendType::Daytona,
            Self::Runloop => BackendType::Runloop,
            Self::E2B => BackendType::E2B,
            Self::AgentComputer => BackendType::AgentComputer,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Daytona => "daytona",
            Self::Runloop => "runloop",
            Self::E2B => "e2b",
            Self::AgentComputer => "agentcomputer",
        }
    }

    fn from_provider_name(name: &str) -> Option<Self> {
        match name {
            "daytona" => Some(Self::Daytona),
            "runloop" => Some(Self::Runloop),
            "e2b" => Some(Self::E2B),
            "agentcomputer" => Some(Self::AgentComputer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BridgeRequest {
    operation: String,
    sandbox_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_namespace: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    remote_metadata: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_revision: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    endpoints: Vec<ResolvedEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vcpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    read_only: Option<bool>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<PortMapping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recursive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct BridgeResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    remote_id: Option<String>,
    #[serde(default)]
    remote_namespace: Option<String>,
    #[serde(default)]
    remote_metadata: HashMap<String, String>,
    #[serde(default)]
    workspace_revision: Option<String>,
    #[serde(default)]
    endpoints: Vec<ResolvedEndpoint>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    running: Option<bool>,
}

#[derive(Debug, Clone)]
struct RemoteBridgeClient {
    provider: RemoteProvider,
}

impl RemoteBridgeClient {
    fn new(provider: RemoteProvider) -> Self {
        Self { provider }
    }

    async fn request(&self, request: &BridgeRequest) -> Result<BridgeResponse> {
        let payload = STANDARD.encode(
            serde_json::to_vec(request).context("Failed to serialize remote bridge request")?,
        );

        let (program, mut args) = remote_bridge_command()?;
        args.push(self.provider.as_str().to_string());
        args.push(payload);

        let output = Command::new(program)
            .args(args)
            .envs(remote_bridge_env(self.provider.as_str()))
            .output()
            .await
            .context("Failed to execute remote bridge")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let message = if !stderr.is_empty() { stderr } else { stdout };
            bail!(
                "Remote bridge request failed for {}: {}",
                self.provider.as_str(),
                message
            );
        }

        let stdout = String::from_utf8(output.stdout).context("Remote bridge returned non-utf8")?;
        let response: BridgeResponse = serde_json::from_str(stdout.trim()).with_context(|| {
            format!(
                "Failed to parse remote bridge response for {}",
                self.provider.as_str()
            )
        })?;

        if !response.success {
            bail!(
                "{} backend error: {}",
                self.provider.as_str(),
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            );
        }

        Ok(response)
    }

    async fn attach(&self, request: &BridgeRequest, env: &HashMap<String, String>) -> Result<i32> {
        let payload = STANDARD.encode(
            serde_json::to_vec(request).context("Failed to serialize remote attach request")?,
        );

        let (program, mut args) = remote_bridge_command()?;
        args.push(self.provider.as_str().to_string());
        args.push(payload);

        let mut command = Command::new(program);
        command.args(args);
        command.stdin(Stdio::inherit());
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::inherit());
        command.envs(remote_bridge_env(self.provider.as_str()));
        command.envs(env.iter());

        let status = command
            .status()
            .await
            .context("Failed to launch remote attach bridge")?;
        Ok(status.code().unwrap_or(-1))
    }
}

pub(crate) fn remote_bridge_command() -> Result<(String, Vec<String>)> {
    if let Ok(custom) = std::env::var("AGENTKERNEL_REMOTE_BRIDGE") {
        if custom.ends_with(".js") || custom.ends_with(".mjs") {
            return Ok(("node".to_string(), vec![custom]));
        }
        return Ok((custom, Vec::new()));
    }

    if let Some(config) = load_project_config()
        && let Some(custom) = config.remote.bridge
    {
        if custom.ends_with(".js") || custom.ends_with(".mjs") {
            return Ok(("node".to_string(), vec![custom]));
        }
        return Ok((custom, Vec::new()));
    }

    let script = resolve_default_bridge_path()?;
    Ok((
        "node".to_string(),
        vec![script.to_string_lossy().to_string()],
    ))
}

pub(crate) fn remote_bridge_env(provider_name: &str) -> HashMap<String, String> {
    let Some(config) = load_project_config() else {
        return HashMap::new();
    };

    let Some(provider) = RemoteProvider::from_provider_name(provider_name) else {
        return HashMap::new();
    };

    provider_bridge_env(provider, &config)
}

fn load_project_config() -> Option<Config> {
    let path = PathBuf::from("agentkernel.toml");
    if !path.exists() {
        return None;
    }
    Config::from_file(&path).ok()
}

fn provider_bridge_env(provider: RemoteProvider, config: &Config) -> HashMap<String, String> {
    let provider_config = match provider {
        RemoteProvider::Daytona => &config.remote.daytona,
        RemoteProvider::Runloop => &config.remote.runloop,
        RemoteProvider::E2B => &config.remote.e2b,
        RemoteProvider::AgentComputer => &config.remote.agentcomputer,
    };

    match provider {
        RemoteProvider::Daytona => daytona_bridge_env(provider_config),
        RemoteProvider::Runloop | RemoteProvider::E2B | RemoteProvider::AgentComputer => {
            HashMap::new()
        }
    }
}

fn daytona_bridge_env(config: &RemoteProviderConfig) -> HashMap<String, String> {
    let mut env = HashMap::new();

    if let Some(api_key) = resolve_provider_secret(config) {
        env.insert("DAYTONA_API_KEY".to_string(), api_key);
    }
    if let Some(base_url) = &config.base_url {
        env.insert("DAYTONA_API_URL".to_string(), base_url.clone());
    }
    if let Some(organization) = &config.organization {
        env.insert("DAYTONA_ORGANIZATION_ID".to_string(), organization.clone());
    }
    if let Some(region) = &config.region {
        env.insert("DAYTONA_TARGET".to_string(), region.clone());
    }

    env
}

fn resolve_provider_secret(config: &RemoteProviderConfig) -> Option<String> {
    if let Some(api_key) = &config.api_key {
        return Some(api_key.clone());
    }

    config
        .api_key_env
        .as_ref()
        .and_then(|key| std::env::var(key).ok())
}

fn resolve_default_bridge_path() -> Result<PathBuf> {
    let candidates = [
        PathBuf::from("scripts/remote-bridge.mjs"),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("../scripts/remote-bridge.mjs")))
            .unwrap_or_default(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("scripts/remote-bridge.mjs")))
            .unwrap_or_default(),
    ];

    for candidate in candidates {
        if !candidate.as_os_str().is_empty() && candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "Remote bridge script not found. Set AGENTKERNEL_REMOTE_BRIDGE or ensure scripts/remote-bridge.mjs exists."
    )
}

pub fn remote_bridge_available() -> bool {
    if remote_bridge_command().is_err() {
        return false;
    }

    std::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub struct RemoteSandbox {
    name: String,
    provider: RemoteProvider,
    bridge: RemoteBridgeClient,
    remote_id: Option<String>,
    remote_namespace: Option<String>,
    remote_metadata: HashMap<String, String>,
    workspace_revision: Option<String>,
    endpoints: Vec<ResolvedEndpoint>,
    running: bool,
    local_workspace: Option<String>,
}

impl RemoteSandbox {
    pub fn new(provider: RemoteProvider, name: &str, state: RemoteSandboxContext) -> Self {
        let running = state
            .remote_metadata
            .get("last_known_status")
            .is_some_and(|value| value == "running");

        Self {
            name: name.to_string(),
            provider,
            bridge: RemoteBridgeClient::new(provider),
            remote_id: state.remote_id,
            remote_namespace: state.remote_namespace,
            remote_metadata: state.remote_metadata,
            workspace_revision: state.workspace_revision,
            endpoints: state.endpoints,
            running,
            local_workspace: state.local_workspace,
        }
    }

    fn profile_for_config(config: &SandboxConfig) -> Option<String> {
        if config.image.trim().is_empty() {
            None
        } else {
            Some(config.image.clone())
        }
    }

    fn env_map(pairs: &[(String, String)]) -> HashMap<String, String> {
        pairs.iter().cloned().collect()
    }

    fn env_strings_map(values: &[String]) -> HashMap<String, String> {
        values
            .iter()
            .filter_map(|entry| {
                entry
                    .split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect()
    }

    fn new_request(&self, operation: &str) -> BridgeRequest {
        BridgeRequest {
            operation: operation.to_string(),
            sandbox_name: self.name.clone(),
            remote_id: self.remote_id.clone(),
            remote_namespace: self.remote_namespace.clone(),
            remote_metadata: self.remote_metadata.clone(),
            workspace_revision: self.workspace_revision.clone(),
            endpoints: self.endpoints.clone(),
            profile: None,
            image: None,
            vcpus: None,
            memory_mb: None,
            network: None,
            read_only: None,
            env: HashMap::new(),
            ports: Vec::new(),
            command: None,
            workdir: None,
            path: None,
            content_base64: None,
            recursive: None,
            local_path: None,
            snapshot_name: None,
            shell: None,
        }
    }

    fn apply_response(&mut self, response: &BridgeResponse) {
        if let Some(remote_id) = &response.remote_id {
            self.remote_id = Some(remote_id.clone());
        }
        if let Some(remote_namespace) = &response.remote_namespace {
            self.remote_namespace = Some(remote_namespace.clone());
        }
        if !response.remote_metadata.is_empty() {
            self.remote_metadata = response.remote_metadata.clone();
        }
        if let Some(workspace_revision) = &response.workspace_revision {
            self.workspace_revision = Some(workspace_revision.clone());
        }
        if !response.endpoints.is_empty() {
            self.endpoints = response.endpoints.clone();
        }
        if let Some(running) = response.running {
            self.running = running;
            self.remote_metadata.insert(
                "last_known_status".to_string(),
                if running { "running" } else { "stopped" }.to_string(),
            );
        }
    }

    async fn sync_push(&mut self) -> Result<()> {
        let Some(local_workspace) = self.local_workspace.clone() else {
            return Ok(());
        };

        let mut request = self.new_request("sync_push");
        request.local_path = Some(local_workspace);
        request.path = Some("/workspace".to_string());
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }

    async fn sync_pull(&mut self) -> Result<()> {
        let Some(local_workspace) = self.local_workspace.clone() else {
            return Ok(());
        };

        let mut request = self.new_request("sync_pull");
        request.local_path = Some(local_workspace);
        request.path = Some("/workspace".to_string());
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }

    async fn refresh_status(&mut self) -> Result<()> {
        let request = self.new_request("status");
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }
}

#[async_trait]
impl Sandbox for RemoteSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        self.local_workspace = config.mount_cwd.then(|| {
            config
                .work_dir
                .clone()
                .unwrap_or_else(|| "/workspace".to_string())
        });

        let mut request = self.new_request(if self.remote_id.is_some() {
            "resume"
        } else {
            "create"
        });
        request.profile = Self::profile_for_config(config);
        request.image = Some(config.image.clone());
        request.vcpus = Some(config.vcpus);
        request.memory_mb = Some(config.memory_mb);
        request.network = Some(config.network);
        request.read_only = Some(config.read_only);
        request.env = Self::env_map(&config.env);
        request.ports = config.ports.clone();
        request.path = Some("/workspace".to_string());

        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);

        if let Some(snapshot_handle) = self.remote_metadata.get("restore_snapshot").cloned() {
            let mut restore = self.new_request("restore");
            restore.snapshot_name = Some(snapshot_handle);
            let restored = self.bridge.request(&restore).await?;
            self.apply_response(&restored);
            self.remote_metadata.remove("restore_snapshot");
        }

        self.sync_push().await?;
        self.running = true;
        self.remote_metadata
            .insert("last_known_status".to_string(), "running".to_string());
        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_options(
            cmd,
            &ExecOptions {
                env: Vec::new(),
                workdir: None,
                user: None,
            },
        )
        .await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        self.exec_with_options(
            cmd,
            &ExecOptions {
                env: env.to_vec(),
                workdir: None,
                user: None,
            },
        )
        .await
    }

    async fn exec_with_options(&mut self, cmd: &[&str], opts: &ExecOptions) -> Result<ExecResult> {
        self.sync_push().await?;

        let mut request = self.new_request("exec");
        request.command = Some(cmd.iter().map(|part| (*part).to_string()).collect());
        request.workdir = opts.workdir.clone();
        request.env = Self::env_strings_map(&opts.env);
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        self.sync_pull().await?;

        Ok(ExecResult {
            exit_code: response.exit_code.unwrap_or(-1),
            stdout: response.stdout.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
        })
    }

    async fn stop(&mut self) -> Result<()> {
        let _ = self.sync_pull().await;
        let request = self.new_request("stop");
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        self.running = false;
        self.remote_metadata
            .insert("last_known_status".to_string(), "stopped".to_string());
        Ok(())
    }

    async fn remove(&mut self) -> Result<()> {
        let request = self.new_request("destroy");
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        self.provider.backend_type()
    }

    fn is_running(&self) -> bool {
        self.running
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()> {
        let mut request = self.new_request("write_file");
        request.path = Some(path.to_string());
        request.content_base64 = Some(STANDARD.encode(content));
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }

    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>> {
        let mut request = self.new_request("read_file");
        request.path = Some(path.to_string());
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        let payload = response
            .content_base64
            .ok_or_else(|| anyhow::anyhow!("Remote backend returned no file content"))?;
        STANDARD
            .decode(payload)
            .context("Failed to decode remote file content")
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()> {
        let mut request = self.new_request("remove_file");
        request.path = Some(path.to_string());
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }

    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> Result<()> {
        let mut request = self.new_request("mkdir");
        request.path = Some(path.to_string());
        request.recursive = Some(recursive);
        let response = self.bridge.request(&request).await?;
        self.apply_response(&response);
        Ok(())
    }

    async fn attach(&mut self, shell: Option<&str>) -> Result<i32> {
        self.attach_with_env(shell, &[]).await
    }

    async fn attach_with_env(&mut self, shell: Option<&str>, env: &[String]) -> Result<i32> {
        self.sync_push().await?;

        let mut request = self.new_request("attach");
        request.shell = shell.map(|value| value.to_string());
        let env_map = Self::env_strings_map(env);
        let exit_code = self.bridge.attach(&request, &env_map).await?;

        let _ = self.sync_pull().await;
        let _ = self.refresh_status().await;

        Ok(exit_code)
    }

    fn runtime_metadata(&self) -> Option<SandboxRuntimeMetadata> {
        Some(SandboxRuntimeMetadata {
            remote_id: self.remote_id.clone(),
            remote_namespace: self.remote_namespace.clone(),
            remote_metadata: self.remote_metadata.clone(),
            workspace_revision: self.workspace_revision.clone(),
            endpoints: self.endpoints.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_remote_provider_backend_mapping() {
        assert_eq!(RemoteProvider::Daytona.backend_type(), BackendType::Daytona);
        assert_eq!(RemoteProvider::Runloop.backend_type(), BackendType::Runloop);
        assert_eq!(RemoteProvider::E2B.backend_type(), BackendType::E2B);
        assert_eq!(
            RemoteProvider::AgentComputer.backend_type(),
            BackendType::AgentComputer
        );
    }

    #[test]
    fn test_remote_sandbox_runtime_metadata_preserved() {
        let mut remote_metadata = HashMap::new();
        remote_metadata.insert("last_known_status".to_string(), "running".to_string());
        let sandbox = RemoteSandbox::new(
            RemoteProvider::Daytona,
            "demo",
            RemoteSandboxContext {
                remote_id: Some("remote-123".to_string()),
                remote_namespace: Some("demo-ns".to_string()),
                remote_metadata: remote_metadata.clone(),
                workspace_revision: Some("rev-1".to_string()),
                endpoints: vec![ResolvedEndpoint {
                    container_port: 3000,
                    protocol: super::super::PortProtocol::Tcp,
                    url: "https://example.test".to_string(),
                }],
                local_workspace: Some("/tmp/demo".to_string()),
            },
        );

        let runtime = sandbox.runtime_metadata().unwrap();
        assert_eq!(runtime.remote_id.as_deref(), Some("remote-123"));
        assert_eq!(runtime.workspace_revision.as_deref(), Some("rev-1"));
        assert_eq!(runtime.endpoints.len(), 1);
        assert_eq!(runtime.remote_metadata, remote_metadata);
    }

    #[test]
    fn test_daytona_bridge_env_from_config() {
        let config = Config::from_str(
            r#"
            [sandbox]
            name = "remote-app"

            [remote.daytona]
            api_key = "daytona-secret"
            base_url = "https://example.invalid/api"
            organization = "org-123"
            region = "eu"
        "#,
        )
        .unwrap();

        let env = provider_bridge_env(RemoteProvider::Daytona, &config);
        assert_eq!(
            env.get("DAYTONA_API_KEY"),
            Some(&"daytona-secret".to_string())
        );
        assert_eq!(
            env.get("DAYTONA_API_URL"),
            Some(&"https://example.invalid/api".to_string())
        );
        assert_eq!(
            env.get("DAYTONA_ORGANIZATION_ID"),
            Some(&"org-123".to_string())
        );
        assert_eq!(env.get("DAYTONA_TARGET"), Some(&"eu".to_string()));
    }
}
