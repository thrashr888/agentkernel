//! Kubernetes backend implementing the Sandbox trait.
//!
//! Each sandbox is a Kubernetes Pod. start() creates a Pod with `sleep infinity`,
//! exec() runs commands via the K8s exec API (WebSocket), stop() deletes the Pod.
//!
//! Compile with `--features kubernetes` to enable.

#![cfg(feature = "kubernetes")]

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::api::networking::v1::{NetworkPolicy, NetworkPolicySpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{Api, DeleteParams, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config as KubeConfig};
use std::collections::{BTreeMap, HashMap};
use tokio::io::AsyncReadExt;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};
use crate::config::OrchestratorConfig;

/// Kubernetes Pod-based sandbox
pub struct KubernetesSandbox {
    /// Sandbox name
    name: String,
    /// Kubernetes namespace for this sandbox's pod
    namespace: String,
    /// Pod name (set after start())
    pod_name: Option<String>,
    /// Whether the sandbox is running
    running: bool,
    /// Kubernetes API client (initialized lazily on start())
    client: Option<Client>,
    /// Optional runtime class (e.g., "gvisor", "kata")
    runtime_class: Option<String>,
    /// Optional service account for the pod
    service_account: Option<String>,
    /// Node selector labels for scheduling
    node_selector: HashMap<String, String>,
    /// Whether a NetworkPolicy was created (for cleanup)
    network_policy_created: bool,
    /// Whether network is disabled (used to decide on NetworkPolicy)
    network_disabled: bool,
}

impl KubernetesSandbox {
    /// Create a new Kubernetes sandbox from orchestrator configuration
    pub fn new(name: &str, config: &OrchestratorConfig) -> Self {
        Self {
            name: name.to_string(),
            namespace: config.namespace.clone(),
            pod_name: None,
            running: false,
            client: None,
            runtime_class: config.runtime_class.clone(),
            service_account: config.service_account.clone(),
            node_selector: config.node_selector.clone(),
            network_policy_created: false,
            network_disabled: false,
        }
    }

    /// Build the Kubernetes API client
    async fn build_client(config: &OrchestratorConfig) -> Result<Client> {
        // Try in-cluster config first (when running inside K8s)
        if let Ok(config) = KubeConfig::incluster() {
            return Client::try_from(config).context("Failed to create in-cluster K8s client");
        }

        // Fall back to kubeconfig
        let kubeconfig = if let Some(ref path) = config.kubeconfig {
            let expanded = tilde_expand(path);
            Kubeconfig::read_from(expanded).context("Failed to read kubeconfig")?
        } else {
            Kubeconfig::read().context("Failed to read default kubeconfig")?
        };

        let mut options = KubeConfigOptions::default();
        if let Some(ref ctx) = config.context {
            options.context = Some(ctx.clone());
        }

        let kube_config = KubeConfig::from_custom_kubeconfig(kubeconfig, &options)
            .await
            .context("Failed to build K8s config from kubeconfig")?;

        Client::try_from(kube_config).context("Failed to create K8s client")
    }

    /// Generate the pod name for this sandbox
    fn pod_name_for(sandbox_name: &str) -> String {
        // K8s names must be DNS-compatible: lowercase, alphanumeric, hyphens
        let sanitized: String = sandbox_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        format!("agentkernel-{}", sanitized)
    }

    /// Standard labels for all agentkernel-managed pods
    fn pod_labels(sandbox_name: &str) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel.io/sandbox".to_string(),
            sandbox_name.to_string(),
        );
        labels.insert(
            "agentkernel.io/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel.io/pool".to_string(), "active".to_string());
        labels
    }

    /// Build the Pod spec for this sandbox
    fn build_pod_spec(&self, config: &SandboxConfig) -> Pod {
        let pod_name = Self::pod_name_for(&self.name);
        let labels = Self::pod_labels(&self.name);

        // Build container security context
        let mut security_context = k8s_openapi::api::core::v1::SecurityContext {
            privileged: Some(false),
            allow_privilege_escalation: Some(false),
            read_only_root_filesystem: Some(config.read_only),
            run_as_non_root: Some(true),
            run_as_user: Some(1000),
            ..Default::default()
        };

        // Drop all capabilities
        security_context.capabilities = Some(k8s_openapi::api::core::v1::Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            ..Default::default()
        });

        // Resource limits
        let mut resource_limits = BTreeMap::new();
        resource_limits.insert(
            "memory".to_string(),
            k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                "{}Mi",
                config.memory_mb
            )),
        );
        resource_limits.insert(
            "cpu".to_string(),
            k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                "{}m",
                config.vcpus * 1000
            )),
        );

        let resource_requests = BTreeMap::new();

        let resources = k8s_openapi::api::core::v1::ResourceRequirements {
            limits: Some(resource_limits),
            requests: Some(resource_requests),
            ..Default::default()
        };

        // Main container: sleep infinity as entrypoint
        let container = Container {
            name: "sandbox".to_string(),
            image: Some(config.image.clone()),
            command: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                "sleep infinity".to_string(),
            ]),
            security_context: Some(security_context),
            resources: Some(resources),
            stdin: Some(true),
            tty: Some(true),
            ..Default::default()
        };

        // Build node selector
        let node_selector: Option<BTreeMap<String, String>> = if self.node_selector.is_empty() {
            None
        } else {
            Some(
                self.node_selector
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        };

        // Pod spec
        let pod_spec = PodSpec {
            containers: vec![container],
            restart_policy: Some("Never".to_string()),
            automount_service_account_token: Some(false),
            runtime_class_name: self.runtime_class.clone(),
            service_account_name: self.service_account.clone(),
            node_selector,
            ..Default::default()
        };

        Pod {
            metadata: ObjectMeta {
                name: Some(pod_name),
                namespace: Some(self.namespace.clone()),
                labels: Some(labels),
                annotations: Some({
                    let mut ann = BTreeMap::new();
                    ann.insert(
                        "pod-security.kubernetes.io/enforce".to_string(),
                        "restricted".to_string(),
                    );
                    ann
                }),
                ..Default::default()
            },
            spec: Some(pod_spec),
            ..Default::default()
        }
    }

    /// Create a NetworkPolicy that denies all ingress/egress for this pod
    async fn create_network_policy(&self, client: &Client) -> Result<()> {
        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &self.namespace);

        let pod_name = Self::pod_name_for(&self.name);
        let np_name = format!("{}-deny-all", pod_name);

        let mut match_labels = BTreeMap::new();
        match_labels.insert("agentkernel.io/sandbox".to_string(), self.name.clone());

        let np = NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(np_name),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                pod_selector: LabelSelector {
                    match_labels: Some(match_labels),
                    ..Default::default()
                },
                // Empty ingress and egress = deny all
                ingress: Some(vec![]),
                egress: Some(vec![]),
                policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
            }),
        };

        np_api
            .create(&PostParams::default(), &np)
            .await
            .context("Failed to create NetworkPolicy")?;

        Ok(())
    }

    /// Delete the NetworkPolicy for this sandbox
    async fn delete_network_policy(&self, client: &Client) -> Result<()> {
        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &self.namespace);
        let pod_name = Self::pod_name_for(&self.name);
        let np_name = format!("{}-deny-all", pod_name);

        let _ = np_api.delete(&np_name, &DeleteParams::default()).await;
        Ok(())
    }

    /// Wait for the pod to reach the Running phase
    async fn wait_for_running(&self, client: &Client, pod_name: &str) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        // Poll for up to 120 seconds
        for _ in 0..240 {
            let pod = pods.get(pod_name).await?;
            if let Some(status) = &pod.status {
                if let Some(phase) = &status.phase {
                    match phase.as_str() {
                        "Running" => return Ok(()),
                        "Failed" | "Succeeded" => {
                            bail!("Pod entered unexpected phase: {}", phase);
                        }
                        _ => {} // Pending, etc.
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        bail!("Timed out waiting for pod '{}' to start", pod_name);
    }
}

#[async_trait]
impl Sandbox for KubernetesSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        // Build K8s client
        let orch_config = OrchestratorConfig {
            namespace: self.namespace.clone(),
            ..Default::default()
        };
        let client = Self::build_client(&orch_config).await?;

        // Ensure namespace exists (best effort)
        let ns_api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(client.clone());
        let _ = ns_api
            .create(
                &PostParams::default(),
                &k8s_openapi::api::core::v1::Namespace {
                    metadata: ObjectMeta {
                        name: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await; // Ignore error if already exists

        // Build and create the pod
        let pod = self.build_pod_spec(config);
        let pod_name = pod
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| Self::pod_name_for(&self.name));

        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        pods.create(&PostParams::default(), &pod)
            .await
            .context("Failed to create K8s pod")?;

        // Create NetworkPolicy if network is disabled
        self.network_disabled = !config.network;
        if !config.network {
            self.create_network_policy(&client).await?;
            self.network_policy_created = true;
        }

        // Wait for the pod to be running
        self.wait_for_running(&client, &pod_name).await?;

        self.pod_name = Some(pod_name);
        self.client = Some(client);
        self.running = true;

        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_env(cmd, &[]).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("K8s client not initialized"))?;
        let pod_name = self
            .pod_name
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Pod not started"))?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        // Wrap command with env if provided
        let full_cmd: Vec<String> = if env.is_empty() {
            cmd.iter().map(|s| s.to_string()).collect()
        } else {
            // Build: env KEY=VAL KEY2=VAL2 ... <original command>
            let mut parts = vec!["env".to_string()];
            parts.extend(env.iter().cloned());
            parts.extend(cmd.iter().map(|s| s.to_string()));
            parts
        };

        let cmd_refs: Vec<&str> = full_cmd.iter().map(|s| s.as_str()).collect();

        // Use the kube API for pod exec via WebSocket
        let attached = pods
            .exec(
                pod_name,
                &cmd_refs,
                &kube::api::AttachParams::default()
                    .container("sandbox")
                    .stdout(true)
                    .stderr(true),
            )
            .await
            .context("Failed to exec in K8s pod")?;

        // Read stdout and stderr concurrently
        let mut stdout_reader = attached
            .stdout()
            .ok_or_else(|| anyhow::anyhow!("No stdout"))?;
        let mut stderr_reader = attached
            .stderr()
            .ok_or_else(|| anyhow::anyhow!("No stderr"))?;

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();

        let (stdout_result, stderr_result) = tokio::join!(
            stdout_reader.read_to_end(&mut stdout_buf),
            stderr_reader.read_to_end(&mut stderr_buf),
        );

        stdout_result.context("Failed to read stdout")?;
        stderr_result.context("Failed to read stderr")?;

        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

        // Try to get exit code from the attached process
        let exit_code = attached
            .join()
            .await
            .map(|statuses| {
                statuses
                    .first()
                    .and_then(|s| s.status.as_ref())
                    .map(|status| if status == "Success" { 0 } else { 1 })
                    .unwrap_or(1)
            })
            .unwrap_or(0);

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    async fn stop(&mut self) -> Result<()> {
        if let (Some(client), Some(pod_name)) = (&self.client, &self.pod_name) {
            let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

            // Delete the pod
            let _ = pods
                .delete(pod_name, &DeleteParams::default())
                .await
                .context("Failed to delete K8s pod");

            // Clean up NetworkPolicy if we created one
            if self.network_policy_created {
                let _ = self.delete_network_policy(client).await;
            }
        }

        self.running = false;
        self.pod_name = None;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Kubernetes
    }

    fn is_running(&self) -> bool {
        self.running
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);

        // Create parent directory first
        if let Some(parent) = std::path::Path::new(path).parent() {
            let parent_str = parent.to_string_lossy();
            if parent_str != "/" {
                let mkdir_cmd = format!("mkdir -p '{}'", parent_str);
                self.exec(&["sh", "-c", &mkdir_cmd]).await?;
            }
        }

        // Decode base64 into the file
        let write_cmd = format!("echo '{}' | base64 -d > '{}'", encoded, path);
        let result = self.exec(&["sh", "-c", &write_cmd]).await?;

        if !result.is_success() {
            bail!("Failed to write file {}: {}", path, result.stderr);
        }

        Ok(())
    }

    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>> {
        let read_cmd = format!("base64 '{}'", path);
        let result = self.exec(&["sh", "-c", &read_cmd]).await?;

        if !result.is_success() {
            bail!("Failed to read file {}: {}", path, result.stderr);
        }

        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(result.stdout.trim())
            .context("Failed to decode base64 file content")?;

        Ok(decoded)
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()> {
        let rm_cmd = format!("rm -f '{}'", path);
        self.exec(&["sh", "-c", &rm_cmd]).await?;
        Ok(())
    }

    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> Result<()> {
        let flag = if recursive { "-p" } else { "" };
        let cmd = format!("mkdir {} '{}'", flag, path);
        self.exec(&["sh", "-c", &cmd]).await?;
        Ok(())
    }

    async fn attach(&mut self, shell: Option<&str>) -> Result<i32> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("K8s client not initialized"))?;
        let pod_name = self
            .pod_name
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Pod not started"))?;

        let shell = shell.unwrap_or("/bin/sh");
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        let attached = pods
            .exec(
                pod_name,
                &[shell],
                &kube::api::AttachParams::default()
                    .container("sandbox")
                    .stdin(true)
                    .stdout(true)
                    .stderr(true)
                    .tty(true),
            )
            .await
            .context("Failed to attach to K8s pod")?;

        // Bridge stdin/stdout for interactive use
        let mut stdin_writer = attached
            .stdin()
            .ok_or_else(|| anyhow::anyhow!("No stdin"))?;
        let mut stdout_reader = attached
            .stdout()
            .ok_or_else(|| anyhow::anyhow!("No stdout"))?;

        let stdin_handle = tokio::spawn(async move {
            let mut host_stdin = tokio::io::stdin();
            let _ = tokio::io::copy(&mut host_stdin, &mut stdin_writer).await;
        });

        let stdout_handle = tokio::spawn(async move {
            let mut host_stdout = tokio::io::stdout();
            let _ = tokio::io::copy(&mut stdout_reader, &mut host_stdout).await;
        });

        // Wait for either to finish
        tokio::select! {
            _ = stdin_handle => {},
            _ = stdout_handle => {},
        }

        Ok(0)
    }

    async fn inject_files(&mut self, files: &[super::FileInjection]) -> Result<()> {
        for file in files {
            // Create parent directory if needed
            if let Some(parent) = std::path::Path::new(&file.dest).parent() {
                let parent_str = parent.to_string_lossy();
                if parent_str != "/" {
                    self.mkdir(&parent_str, true).await?;
                }
            }
            // Write the file
            self.write_file(&file.dest, &file.content).await?;
        }
        Ok(())
    }
}

/// Expand tilde (~) to home directory in paths
fn tilde_expand(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}{}", home.to_string_lossy(), &path[1..]);
        }
    }
    path.to_string()
}
