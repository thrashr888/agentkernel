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
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use tokio::io::AsyncReadExt;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};
use crate::config::OrchestratorConfig;

const KUBERNETES_NAME_MAX_LEN: usize = 63;
const KUBERNETES_HASH_LEN: usize = 12;

/// Names and labels used to identify all resources belonging to one sandbox.
///
/// Sandbox names are validated by the public API, but the validator permits
/// underscores and uppercase characters that Kubernetes resource names do not.
/// A hash is therefore added only when the old normalized name would be
/// ambiguous or too long. This keeps common resource names stable while making
/// names such as a_b and a-b distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KubernetesResourceIdentity {
    sandbox_label: String,
    pod_name: String,
    legacy_pod_name: String,
    service_name: String,
    legacy_service_name: String,
    network_policy_name: String,
    legacy_network_policy_name: String,
}

impl KubernetesResourceIdentity {
    fn for_sandbox(sandbox_name: &str) -> Self {
        let legacy_pod_name = legacy_pod_name_for(sandbox_name);
        let legacy_service_name = legacy_service_name_for(sandbox_name);
        let legacy_network_policy_name = format!("{legacy_pod_name}-deny-all");

        Self {
            sandbox_label: sandbox_label_for(sandbox_name),
            pod_name: canonical_dns_name(sandbox_name, "agentkernel-", ""),
            legacy_pod_name,
            service_name: canonical_dns_name(sandbox_name, "agentkernel-", "-svc"),
            legacy_service_name,
            network_policy_name: canonical_dns_name(sandbox_name, "agentkernel-", "-deny-all"),
            legacy_network_policy_name,
        }
    }

    fn pod_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.pod_name.as_str()).chain(
            std::iter::once(self.legacy_pod_name.as_str()).filter(|name| *name != self.pod_name),
        )
    }

    fn service_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.service_name.as_str()).chain(
            std::iter::once(self.legacy_service_name.as_str())
                .filter(|name| *name != self.service_name),
        )
    }

    fn network_policy_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.network_policy_name.as_str()).chain(
            std::iter::once(self.legacy_network_policy_name.as_str())
                .filter(|name| *name != self.network_policy_name),
        )
    }
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..(KUBERNETES_HASH_LEN / 2)]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= KUBERNETES_NAME_MAX_LEN
        && value.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && value.ends_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn is_label_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= KUBERNETES_NAME_MAX_LEN
        && value.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        && value.ends_with(|ch: char| ch.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn sandbox_label_for(sandbox_name: &str) -> String {
    if is_label_value(sandbox_name) {
        sandbox_name.to_string()
    } else {
        format!("agentkernel-{}", short_hash(sandbox_name))
    }
}

fn canonical_dns_name(sandbox_name: &str, prefix: &str, suffix: &str) -> String {
    let normalized: String = sandbox_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let normalized = normalized.trim_matches('-');
    let common = format!("{prefix}{normalized}{suffix}");
    if normalized == sandbox_name.to_ascii_lowercase() && is_dns_label(&common) {
        return common;
    }

    let hash = short_hash(sandbox_name);
    let available = KUBERNETES_NAME_MAX_LEN
        .saturating_sub(prefix.len())
        .saturating_sub(suffix.len())
        .saturating_sub(hash.len() + 1);
    let base: String = normalized
        .chars()
        .take(available)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() { "sandbox" } else { &base };
    let name = format!("{prefix}{base}-{hash}{suffix}");
    debug_assert!(is_dns_label(&name));
    name
}

fn legacy_pod_name_for(sandbox_name: &str) -> String {
    let sanitized: String = sandbox_name
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    format!("agentkernel-{sanitized}")
}

fn legacy_service_name_for(sandbox_name: &str) -> String {
    let sanitized: String = sandbox_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
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
    let max_base_len = KUBERNETES_NAME_MAX_LEN - "agentkernel-".len() - suffix.len();
    let base: String = base.chars().take(max_base_len).collect();
    format!("agentkernel-{}{}", base.trim_matches('-'), suffix)
}

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
        KubernetesResourceIdentity::for_sandbox(sandbox_name).pod_name
    }

    /// Standard labels for all agentkernel-managed pods
    fn pod_labels(sandbox_name: &str) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel/sandbox".to_string(),
            KubernetesResourceIdentity::for_sandbox(sandbox_name).sandbox_label,
        );
        labels.insert(
            "agentkernel/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel/pool".to_string(), "active".to_string());
        labels
    }

    /// Generate a deterministic DNS-safe Service name for this sandbox.
    fn service_name_for(sandbox_name: &str) -> String {
        KubernetesResourceIdentity::for_sandbox(sandbox_name).service_name
    }

    fn identity(&self) -> KubernetesResourceIdentity {
        KubernetesResourceIdentity::for_sandbox(&self.name)
    }

    fn owned_labels(
        labels: Option<&BTreeMap<String, String>>,
        sandbox_name: &str,
        allow_legacy: bool,
    ) -> bool {
        let Some(labels) = labels else {
            return false;
        };
        if labels.get("agentkernel/managed-by").map(String::as_str) != Some("agentkernel") {
            return false;
        }

        let identity = KubernetesResourceIdentity::for_sandbox(sandbox_name);
        labels.get("agentkernel/sandbox").is_some_and(|value| {
            value == &identity.sandbox_label || (allow_legacy && value == sandbox_name)
        })
    }

    fn network_policy_owned(policy: &NetworkPolicy, sandbox_name: &str) -> bool {
        let Some(selector) = policy
            .spec
            .as_ref()
            .and_then(|spec| spec.pod_selector.as_ref())
            .and_then(|selector| selector.match_labels.as_ref())
            .and_then(|labels| labels.get("agentkernel/sandbox"))
        else {
            return false;
        };
        let identity = KubernetesResourceIdentity::for_sandbox(sandbox_name);
        let labels = policy.metadata.labels.as_ref();
        let canonical_owned =
            Self::owned_labels(labels, sandbox_name, false) && selector == &identity.sandbox_label;
        let legacy_owned = labels.is_none_or(BTreeMap::is_empty) && selector == sandbox_name;
        canonical_owned || legacy_owned
    }

    async fn resolve_pod_name(&self, client: &Client) -> Result<String> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        let identity = self.identity();
        for candidate in identity.pod_names() {
            if let Ok(pod) = pods.get(candidate).await
                && Self::owned_labels(pod.metadata.labels.as_ref(), &self.name, true)
            {
                return Ok(candidate.to_string());
            }
        }
        bail!(
            "No AgentKernel-owned Kubernetes pod found for sandbox '{}'",
            self.name
        )
    }

    async fn delete_owned_pods(&self, client: &Client) {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        let identity = self.identity();
        for name in identity.pod_names() {
            if let Ok(pod) = pods.get(name).await
                && Self::owned_labels(pod.metadata.labels.as_ref(), &self.name, true)
            {
                let _ = pods.delete(name, &DeleteParams::default()).await;
            }
        }
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
        let identity = self.identity();
        labels.insert(
            "agentkernel/sandbox".to_string(),
            identity.sandbox_label.clone(),
        );

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
                    selector.insert("agentkernel/sandbox".to_string(), identity.sandbox_label);
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
        let identity = self.identity();
        for name in identity.service_names() {
            // Reconnects do not retain service_created. Only delete resources
            // carrying AgentKernel ownership labels, never a colliding
            // user-owned Service with the same deterministic name.
            if let Ok(service) = services.get(name).await
                && Self::owned_labels(service.metadata.labels.as_ref(), &self.name, true)
            {
                let _ = services.delete(name, &DeleteParams::default()).await;
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
        let identity = self.identity();

        let mut match_labels = BTreeMap::new();
        match_labels.insert(
            "agentkernel/sandbox".to_string(),
            identity.sandbox_label.clone(),
        );
        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel/sandbox".to_string(), identity.sandbox_label);

        NetworkPolicy {
            metadata: ObjectMeta {
                name: Some(identity.network_policy_name),
                namespace: Some(self.namespace.clone()),
                labels: Some(labels),
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
        let identity = self.identity();
        for name in identity.network_policy_names() {
            if let Ok(policy) = np_api.get(name).await
                && Self::network_policy_owned(&policy, &self.name)
            {
                let _ = np_api.delete(name, &DeleteParams::default()).await;
            }
        }
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
        // Lazily initialize the client and resolve the canonical or legacy pod
        // (e.g., reconnecting to a running pod after an upgrade).
        if self.client.is_none() {
            let orch_config = OrchestratorConfig {
                namespace: self.namespace.clone(),
                ..Default::default()
            };
            let client = Self::build_client(&orch_config).await?;
            self.client = Some(client);
        }
        let client = self.client.clone().unwrap();
        let pod_name = if let Some(pod_name) = self.pod_name.clone() {
            pod_name
        } else {
            let pod_name = self.resolve_pod_name(&client).await?;
            self.pod_name = Some(pod_name.clone());
            pod_name
        };

        let pods: Api<Pod> = Api::namespaced(client, &self.namespace);

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
                &pod_name,
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
        // Lazily initialize the client. Resource lookup below verifies
        // ownership before acting on either canonical or legacy names.
        if self.client.is_none() {
            let orch_config = OrchestratorConfig {
                namespace: self.namespace.clone(),
                ..Default::default()
            };
            if let Ok(client) = Self::build_client(&orch_config).await {
                self.client = Some(client);
            }
        }

        if let Some(client) = self.client.clone() {
            // Delete every matching AgentKernel-owned pod. This also cleans
            // up both names if a migration left canonical and legacy pods.
            self.delete_owned_pods(&client).await;

            // Clean up the NetworkPolicy, including after reconnecting in a
            // new process where the in-memory creation flag is unavailable.
            let _ = self.delete_network_policy(&client).await;
            self.network_policy_created = false;

            // Clean up the internal Service, including after reconnecting in a
            // new process where the in-memory creation flag is unavailable.
            let _ = self.delete_service(&client).await;
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
            .clone()
            .ok_or_else(|| anyhow::anyhow!("K8s client not initialized"))?;
        let pod_name = if let Some(pod_name) = self.pod_name.clone() {
            pod_name
        } else {
            let pod_name = self.resolve_pod_name(&client).await?;
            self.pod_name = Some(pod_name.clone());
            pod_name
        };

        let shell = shell.unwrap_or("/bin/sh");
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        let mut attached = pods
            .exec(
                &pod_name,
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
        let sandbox = KubernetesSandbox::new("test-sandbox", &OrchestratorConfig::default());
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
            Some("test-sandbox")
        );
        assert_eq!(
            policy
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("agentkernel/sandbox"))
                .map(String::as_str),
            Some("test-sandbox")
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
            KubernetesSandbox::new("my-sandbox-production", &OrchestratorConfig::default());
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
            Some("my-sandbox-production")
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
        let identity = KubernetesResourceIdentity::for_sandbox(&"a".repeat(63));

        for name in [
            identity.pod_name,
            identity.service_name,
            identity.network_policy_name,
        ] {
            assert!(is_dns_label(&name), "not a DNS label: {name}");
            assert!(name.is_ascii());
            assert!(name.len() <= 63);
        }
    }

    #[test]
    fn normalized_names_are_collision_resistant() {
        let underscored = KubernetesResourceIdentity::for_sandbox("a_b");
        let hyphenated = KubernetesResourceIdentity::for_sandbox("a-b");

        assert_ne!(underscored.pod_name, hyphenated.pod_name);
        assert_ne!(underscored.service_name, hyphenated.service_name);
        assert_ne!(
            underscored.network_policy_name,
            hyphenated.network_policy_name
        );
        assert_eq!(hyphenated.pod_name, "agentkernel-a-b");
        assert_eq!(hyphenated.service_name, "agentkernel-a-b-svc");
        assert_eq!(hyphenated.network_policy_name, "agentkernel-a-b-deny-all");
    }

    #[test]
    fn common_names_remain_stable_and_labels_preserve_valid_input() {
        let identity = KubernetesResourceIdentity::for_sandbox("my-sandbox-1");

        assert_eq!(identity.pod_name, "agentkernel-my-sandbox-1");
        assert_eq!(identity.service_name, "agentkernel-my-sandbox-1-svc");
        assert_eq!(
            identity.network_policy_name,
            "agentkernel-my-sandbox-1-deny-all"
        );
        assert_eq!(identity.sandbox_label, "my-sandbox-1");
        assert!(is_label_value(&identity.sandbox_label));
    }

    #[test]
    fn uppercase_names_keep_legacy_lowercase_resource_names() {
        let identity = KubernetesResourceIdentity::for_sandbox("MySandbox");

        assert_eq!(identity.pod_name, "agentkernel-mysandbox");
        assert_eq!(identity.service_name, "agentkernel-mysandbox-svc");
        assert_eq!(
            identity.network_policy_name,
            "agentkernel-mysandbox-deny-all"
        );
    }

    #[test]
    fn legacy_candidates_and_ownership_checks_are_deterministic() {
        let identity = KubernetesResourceIdentity::for_sandbox("a_b");
        assert_eq!(
            identity.pod_names().collect::<Vec<_>>(),
            vec![
                identity.pod_name.as_str(),
                identity.legacy_pod_name.as_str()
            ]
        );
        assert!(KubernetesSandbox::owned_labels(
            Some(&BTreeMap::from([
                (
                    "agentkernel/managed-by".to_string(),
                    "agentkernel".to_string()
                ),
                ("agentkernel/sandbox".to_string(), "a_b".to_string()),
            ])),
            "a_b",
            true
        ));
        assert!(!KubernetesSandbox::owned_labels(
            Some(&BTreeMap::from([
                (
                    "agentkernel/managed-by".to_string(),
                    "agentkernel".to_string()
                ),
                ("agentkernel/sandbox".to_string(), "a-b".to_string()),
            ])),
            "a_b",
            true
        ));
    }

    #[test]
    fn network_policy_legacy_ownership_requires_exact_selector() {
        let sandbox = KubernetesSandbox::new("a_b", &OrchestratorConfig::default());
        let mut legacy = sandbox.build_network_policy();
        legacy.metadata.name =
            Some(KubernetesResourceIdentity::for_sandbox("a_b").legacy_network_policy_name);
        legacy.metadata.labels = None;
        legacy
            .spec
            .as_mut()
            .expect("policy spec")
            .pod_selector
            .as_mut()
            .expect("pod selector")
            .match_labels
            .as_mut()
            .expect("match labels")
            .insert("agentkernel/sandbox".to_string(), "a_b".to_string());

        assert!(KubernetesSandbox::network_policy_owned(&legacy, "a_b"));
        legacy
            .spec
            .as_mut()
            .expect("policy spec")
            .pod_selector
            .as_mut()
            .expect("pod selector")
            .match_labels
            .as_mut()
            .expect("match labels")
            .insert("agentkernel/sandbox".to_string(), "a-b".to_string());
        assert!(!KubernetesSandbox::network_policy_owned(&legacy, "a_b"));
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
