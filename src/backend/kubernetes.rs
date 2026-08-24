//! Kubernetes backend implementing the Sandbox trait.
//!
//! Each sandbox is a Kubernetes Pod. start() creates a Pod with `sleep infinity`,
//! exec() runs commands via the K8s exec API (WebSocket), stop() deletes the Pod.
//!
//! Compile with `--features kubernetes` to enable.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, Service, ServicePort, ServiceSpec};
use k8s_openapi::api::networking::v1::{NetworkPolicy, NetworkPolicySpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
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
    /// Whether an internal ClusterIP Service was created (for cleanup)
    service_created: bool,
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
            service_created: false,
            network_disabled: false,
        }
    }

    fn validate_ports(config: &SandboxConfig) -> Result<()> {
        if !config.network && !config.ports.is_empty() {
            bail!(
                "Kubernetes internal Services require network access; cannot declare ports when network is disabled"
            );
        }
        Ok(())
    }

    /// Build the Kubernetes API client
    async fn build_client(config: &OrchestratorConfig) -> Result<Client> {
        // Multiple optional dependencies enable different rustls providers. Select the
        // provider AgentKernel uses before kube builds its HTTPS client so binaries
        // with those feature combinations cannot panic during client construction.
        let _ = rustls::crypto::ring::default_provider().install_default();

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
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        format!("agentkernel-{}", sanitized)
    }

    /// Standard labels for all agentkernel-managed pods
    fn pod_labels(sandbox_name: &str) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert("agentkernel/sandbox".to_string(), sandbox_name.to_string());
        labels.insert(
            "agentkernel/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel/pool".to_string(), "active".to_string());
        labels
    }

    /// Generate a deterministic DNS-safe Service name for this sandbox.
    fn service_name_for(sandbox_name: &str) -> String {
        // Kubernetes Service names are DNS labels: ASCII lowercase, at most 63
        // bytes, and may not start or end with a hyphen.
        let sanitized: String = sandbox_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let sanitized = sanitized.trim_matches('-');
        let base = if sanitized.is_empty() {
            "sandbox"
        } else {
            sanitized
        };
        let suffix = "-svc";
        let max_base_len = 63 - "agentkernel-".len() - suffix.len();
        let base: String = base.chars().take(max_base_len).collect();
        format!("agentkernel-{}{}", base.trim_matches('-'), suffix)
    }

    /// Build the internal ClusterIP Service for the sandbox's declared ports.
    fn build_service(&self, config: &SandboxConfig) -> Service {
        let ports = config
            .ports
            .iter()
            .enumerate()
            .map(|(index, port)| ServicePort {
                // The index makes names unique even when mappings repeat.
                name: Some(format!(
                    "port-{}-{}",
                    index,
                    match port.protocol {
                        super::PortProtocol::Tcp => "tcp",
                        super::PortProtocol::Udp => "udp",
                    }
                )),
                protocol: Some(match port.protocol {
                    super::PortProtocol::Tcp => "TCP".to_string(),
                    super::PortProtocol::Udp => "UDP".to_string(),
                }),
                port: port.host_port.unwrap_or(port.container_port) as i32,
                target_port: Some(IntOrString::Int(port.container_port as i32)),
                ..Default::default()
            })
            .collect();

        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel/sandbox".to_string(), self.name.clone());

        Service {
            metadata: ObjectMeta {
                name: Some(Self::service_name_for(&self.name)),
                namespace: Some(self.namespace.clone()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                type_: Some("ClusterIP".to_string()),
                ports: Some(ports),
                selector: Some({
                    let mut selector = BTreeMap::new();
                    selector.insert("agentkernel/sandbox".to_string(), self.name.clone());
                    selector
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create the internal Service for this sandbox.
    async fn create_service(&self, client: &Client, service: &Service) -> Result<()> {
        let services: Api<Service> = Api::namespaced(client.clone(), &self.namespace);
        services
            .create(&PostParams::default(), service)
            .await
            .context("Failed to create K8s Service")?;
        Ok(())
    }

    /// Delete the internal Service for this sandbox.
    async fn delete_service(&self, client: &Client) -> Result<()> {
        let services: Api<Service> = Api::namespaced(client.clone(), &self.namespace);
        let name = Self::service_name_for(&self.name);
        // Reconnects do not retain service_created. Only delete a resource
        // carrying both AgentKernel ownership labels, never a colliding
        // user-owned Service with the same deterministic name.
        if let Ok(service) = services.get(&name).await {
            let labels = service.metadata.labels.as_ref();
            let owned = labels
                .and_then(|labels| labels.get("agentkernel/managed-by"))
                .is_some_and(|value| value == "agentkernel")
                && labels
                    .and_then(|labels| labels.get("agentkernel/sandbox"))
                    .is_some_and(|value| value == &self.name);
            if owned {
                let _ = services.delete(&name, &DeleteParams::default()).await;
            }
        }
        Ok(())
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

        // Build container port specs
        let container_ports: Option<Vec<k8s_openapi::api::core::v1::ContainerPort>> =
            if config.ports.is_empty() {
                None
            } else {
                Some(
                    config
                        .ports
                        .iter()
                        .map(|pm| k8s_openapi::api::core::v1::ContainerPort {
                            container_port: pm.container_port as i32,
                            protocol: Some(match pm.protocol {
                                super::PortProtocol::Tcp => "TCP".to_string(),
                                super::PortProtocol::Udp => "UDP".to_string(),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                )
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
            ports: container_ports,
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

    /// Build a NetworkPolicy that denies all ingress/egress for this pod.
    fn build_network_policy(&self) -> NetworkPolicy {
        let pod_name = Self::pod_name_for(&self.name);
        let np_name = format!("{}-deny-all", pod_name);

        let mut match_labels = BTreeMap::new();
        match_labels.insert("agentkernel/sandbox".to_string(), self.name.clone());

        NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(np_name),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(match_labels),
                    ..Default::default()
                }),
                // Empty ingress and egress = deny all
                ingress: Some(vec![]),
                egress: Some(vec![]),
                policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
            }),
        }
    }

    /// Create a NetworkPolicy that denies all ingress/egress for this pod
    async fn create_network_policy(&self, client: &Client) -> Result<()> {
        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &self.namespace);
        let np = self.build_network_policy();

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

    /// Wait for the pod to reach the Running phase.
    /// Uses exponential backoff: 50ms → 100ms → 200ms → 500ms (capped).
    async fn wait_for_running(&self, client: &Client, pod_name: &str) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        let mut delay_ms: u64 = 50;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);

        loop {
            let pod = pods.get(pod_name).await?;
            if let Some(status) = &pod.status
                && let Some(phase) = &status.phase
            {
                match phase.as_str() {
                    "Running" => return Ok(()),
                    "Failed" | "Succeeded" => {
                        bail!("Pod entered unexpected phase: {}", phase);
                    }
                    _ => {} // Pending, etc.
                }
            }

            if tokio::time::Instant::now() >= deadline {
                bail!("Timed out waiting for pod '{}' to start", pod_name);
            }

            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(500);
        }
    }
}

#[async_trait]
impl Sandbox for KubernetesSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        Self::validate_ports(config)?;

        // Build K8s client
        let orch_config = OrchestratorConfig {
            namespace: self.namespace.clone(),
            ..Default::default()
        };
        let client = Self::build_client(&orch_config).await?;

        // Ensure namespace exists (only create if missing)
        let ns_api: Api<k8s_openapi::api::core::v1::Namespace> = Api::all(client.clone());
        if ns_api.get(&self.namespace).await.is_err() {
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
                .await;
        }

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
            if let Err(error) = self.create_network_policy(&client).await {
                let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
                return Err(error);
            }
            self.network_policy_created = true;
        }

        // Create the per-sandbox internal Service after the pod and policy. If
        // this fails, roll back every resource created by this start attempt.
        if !config.ports.is_empty() {
            let service = self.build_service(config);
            if let Err(error) = self.create_service(&client, &service).await {
                let _ = self.delete_service(&client).await;
                if self.network_policy_created {
                    let _ = self.delete_network_policy(&client).await;
                    self.network_policy_created = false;
                }
                let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
                return Err(error);
            }
            self.service_created = true;
        }

        // Wait for the pod to be running
        if let Err(error) = self.wait_for_running(&client, &pod_name).await {
            if self.service_created {
                let _ = self.delete_service(&client).await;
                self.service_created = false;
            }
            if self.network_policy_created {
                let _ = self.delete_network_policy(&client).await;
                self.network_policy_created = false;
            }
            let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
            return Err(error);
        }

        self.pod_name = Some(pod_name);
        self.client = Some(client);
        self.running = true;

        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_env(cmd, &[]).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        // Lazily initialize client and pod_name if needed (e.g., reconnecting to a running pod)
        if self.client.is_none() {
            let orch_config = OrchestratorConfig {
                namespace: self.namespace.clone(),
                ..Default::default()
            };
            let client = Self::build_client(&orch_config).await?;
            self.client = Some(client);
        }
        if self.pod_name.is_none() {
            self.pod_name = Some(Self::pod_name_for(&self.name));
        }

        let client = self.client.as_ref().unwrap();
        let pod_name = self.pod_name.as_ref().unwrap();

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

        // Use the kube API for pod exec via WebSocket
        let mut attached = pods
            .exec(
                pod_name,
                full_cmd,
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

        // Wait for the process to complete; infer exit code from stderr
        let _ = attached.join().await;
        let exit_code = if stderr.is_empty() { 0 } else { 1 };

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    async fn stop(&mut self) -> Result<()> {
        // Lazily initialize client and pod_name if needed
        if self.client.is_none() {
            let orch_config = OrchestratorConfig {
                namespace: self.namespace.clone(),
                ..Default::default()
            };
            if let Ok(client) = Self::build_client(&orch_config).await {
                self.client = Some(client);
            }
        }
        if self.pod_name.is_none() {
            self.pod_name = Some(Self::pod_name_for(&self.name));
        }

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
                self.network_policy_created = false;
            }

            // Clean up the internal Service, including after reconnecting in a
            // new process where the in-memory creation flag is unavailable.
            let _ = self.delete_service(client).await;
            self.service_created = false;
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

        let mut attached = pods
            .exec(
                pod_name,
                vec![shell.to_string()],
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
    if path.starts_with("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}{}", home.to_string_lossy(), &path[1..]);
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_all_policy_targets_only_the_sandbox() {
        let sandbox = KubernetesSandbox::new("Test Sandbox", &OrchestratorConfig::default());
        let policy = sandbox.build_network_policy();

        assert_eq!(
            policy.metadata.name.as_deref(),
            Some("agentkernel-test-sandbox-deny-all")
        );
        let spec = policy.spec.expect("network policy spec");
        let selector = spec.pod_selector.expect("pod selector");
        assert_eq!(
            selector
                .match_labels
                .expect("selector labels")
                .get("agentkernel/sandbox")
                .map(String::as_str),
            Some("Test Sandbox")
        );
        assert_eq!(spec.ingress, Some(vec![]));
        assert_eq!(spec.egress, Some(vec![]));
        assert_eq!(
            spec.policy_types,
            Some(vec!["Ingress".to_string(), "Egress".to_string()])
        );
    }

    #[test]
    fn internal_service_has_stable_dns_name_and_pod_selector() {
        let sandbox =
            KubernetesSandbox::new("My Sandbox/Production", &OrchestratorConfig::default());
        let service = sandbox.build_service(&SandboxConfig {
            ports: vec![super::super::PortMapping {
                host_port: Some(18080),
                container_port: 8080,
                protocol: super::super::PortProtocol::Tcp,
            }],
            ..Default::default()
        });

        assert_eq!(
            service.metadata.name.as_deref(),
            Some("agentkernel-my-sandbox-production-svc")
        );
        let spec = service.spec.expect("service spec");
        assert_eq!(spec.type_.as_deref(), Some("ClusterIP"));
        assert_eq!(
            spec.selector
                .expect("service selector")
                .get("agentkernel/sandbox")
                .map(String::as_str),
            Some("My Sandbox/Production")
        );
        let port = &spec.ports.expect("service ports")[0];
        assert_eq!(port.name.as_deref(), Some("port-0-tcp"));
        assert_eq!(port.protocol.as_deref(), Some("TCP"));
        assert_eq!(port.port, 18080);
        assert_eq!(port.target_port, Some(IntOrString::Int(8080)));
    }

    #[test]
    fn internal_service_preserves_udp_and_auto_port_target() {
        let sandbox = KubernetesSandbox::new("udp sandbox", &OrchestratorConfig::default());
        let service = sandbox.build_service(&SandboxConfig {
            ports: vec![
                super::super::PortMapping {
                    host_port: None,
                    container_port: 5353,
                    protocol: super::super::PortProtocol::Udp,
                },
                super::super::PortMapping {
                    host_port: None,
                    container_port: 5353,
                    protocol: super::super::PortProtocol::Udp,
                },
            ],
            ..Default::default()
        });
        let ports = service.spec.expect("service spec").ports.expect("ports");

        assert_eq!(ports[0].name.as_deref(), Some("port-0-udp"));
        assert_eq!(ports[1].name.as_deref(), Some("port-1-udp"));
        assert_eq!(ports[0].protocol.as_deref(), Some("UDP"));
        assert_eq!(ports[0].port, 5353);
        assert_eq!(ports[0].target_port, Some(IntOrString::Int(5353)));
    }

    #[test]
    fn service_name_is_bounded_and_never_empty() {
        let long_name = "---".to_string() + &"a".repeat(200);
        let service_name = KubernetesSandbox::service_name_for(&long_name);

        assert!(service_name.len() <= 63);
        assert!(service_name.starts_with("agentkernel-a"));
        assert!(!service_name.ends_with('-'));
    }

    #[test]
    fn ports_are_rejected_when_network_is_disabled() {
        let config = SandboxConfig {
            network: false,
            ports: vec![super::super::PortMapping {
                host_port: None,
                container_port: 8080,
                protocol: super::super::PortProtocol::Tcp,
            }],
            ..Default::default()
        };

        let error = KubernetesSandbox::validate_ports(&config).expect_err("invalid ports");
        assert!(error.to_string().contains("require network access"));
    }
}
