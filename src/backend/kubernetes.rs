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
use crate::config::{KubernetesVolumeClaim, OrchestratorConfig};

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
    /// Preserve the explicitly selected cluster across start/exec/stop.
    kubeconfig: Option<String>,
    context: Option<String>,
    startup_timeout_secs: u64,
    persistent_volume_claims: Vec<KubernetesVolumeClaim>,
    fs_group: Option<i64>,
    owner_id: Option<String>,
    /// Optional runtime class (e.g., "gvisor", "kata", "isolated-sandbox")
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
            kubeconfig: config.kubeconfig.clone(),
            context: config.context.clone(),
            startup_timeout_secs: config.startup_timeout_secs,
            persistent_volume_claims: config.persistent_volume_claims.clone(),
            fs_group: config.fs_group,
            owner_id: config.kubernetes_owner_id.clone(),
            runtime_class: config.runtime_class.clone(),
            service_account: config.service_account.clone(),
            node_selector: config.node_selector.clone(),
            network_policy_created: false,
            service_created: false,
            network_disabled: false,
        }
    }

    fn client_config(&self) -> OrchestratorConfig {
        OrchestratorConfig {
            namespace: self.namespace.clone(),
            kubeconfig: self.kubeconfig.clone(),
            context: self.context.clone(),
            runtime_class: self.runtime_class.clone(),
            ..Default::default()
        }
    }

    fn validate_configuration(&self) -> Result<()> {
        self.client_config().validate_kubernetes_target()?;
        if self.startup_timeout_secs == 0 || self.startup_timeout_secs > 86_400 {
            bail!("Kubernetes startup_timeout_secs must be between 1 and 86400");
        }
        if self.fs_group.is_some_and(|group| group < 0) {
            bail!("Kubernetes fs_group must not be negative");
        }
        let mut paths = std::collections::HashSet::new();
        for claim in &self.persistent_volume_claims {
            if claim.claim_name.len() > 253 || !claim.claim_name.split('.').all(is_dns_label) {
                bail!("Invalid Kubernetes PVC name: '{}'", claim.claim_name);
            }
            let path = &claim.mount_path;
            if !path.starts_with('/')
                || path == "/"
                || path.contains('\0')
                || path
                    .split('/')
                    .skip(1)
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                bail!(
                    "PVC mount_path must be an absolute non-root path without empty, '.' or '..' components: '{path}'"
                );
            }
            if !paths.insert(path) {
                bail!("Duplicate Kubernetes PVC mount_path: '{path}'");
            }
        }
        Ok(())
    }

    async fn rollback_start(&mut self, error: anyhow::Error) -> anyhow::Error {
        match self.stop().await {
            Ok(()) => error,
            Err(cleanup) => anyhow::anyhow!(
                "{error:#}; Kubernetes rollback failed: {cleanup:#}. Retry stop for sandbox '{}'",
                self.name
            ),
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
        config.validate_kubernetes_target()?;
        // Multiple optional dependencies enable different rustls providers. Select the
        // provider AgentKernel uses before kube builds its HTTPS client so binaries
        // with those feature combinations cannot panic during client construction.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Explicit cluster selection must not silently target the hosting cluster.
        if config.kubeconfig.is_none()
            && config.context.is_none()
            && let Ok(config) = KubeConfig::incluster()
        {
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
    pub(crate) fn pod_labels(sandbox_name: &str) -> BTreeMap<String, String> {
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

    fn owns_labels(&self, labels: Option<&BTreeMap<String, String>>, allow_legacy: bool) -> bool {
        Self::owned_labels(labels, &self.name, allow_legacy)
            && self.owner_id.as_ref().is_none_or(|owner| {
                labels.and_then(|labels| labels.get("agentkernel/owner")) == Some(owner)
            })
    }

    fn owns_network_policy(&self, policy: &NetworkPolicy) -> bool {
        Self::network_policy_owned(policy, &self.name)
            && self.owner_id.as_ref().is_none_or(|owner| {
                policy
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get("agentkernel/owner"))
                    == Some(owner)
            })
    }

    fn add_owner_label(&self, labels: &mut BTreeMap<String, String>) {
        if let Some(owner) = &self.owner_id {
            labels.insert("agentkernel/owner".into(), owner.clone());
        }
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
                && self.owns_labels(pod.metadata.labels.as_ref(), true)
            {
                return Ok(candidate.to_string());
            }
        }
        bail!(
            "No AgentKernel-owned Kubernetes pod found for sandbox '{}'",
            self.name
        )
    }

    async fn delete_owned_pods(&self, client: &Client) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        let identity = self.identity();
        for name in identity.pod_names() {
            if let Some(pod) = pods
                .get_opt(name)
                .await
                .context("Failed to inspect K8s pod for cleanup")?
                && self.owns_labels(pod.metadata.labels.as_ref(), true)
            {
                let params = DeleteParams {
                    preconditions: Some(kube::api::Preconditions {
                        uid: pod.metadata.uid.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                ignore_not_found(pods.delete(name, &params).await)
                    .context("Failed to delete K8s pod")?;
                // Delete acknowledgement is not termination. Do not report stopped
                // while a finalizer or graceful shutdown still retains the pod.
                tokio::time::timeout(std::time::Duration::from_secs(120), async {
                    loop {
                        match pods.get_opt(name).await? {
                            None => return Ok::<(), kube::Error>(()),
                            Some(current) if current.metadata.uid != pod.metadata.uid => {
                                return Ok(());
                            }
                            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                        }
                    }
                })
                .await
                .context("Timed out waiting for Kubernetes pod deletion; stop can be retried")??;
            }
        }
        Ok(())
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

        self.add_owner_label(&mut labels);
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
                    self.add_owner_label(&mut selector);
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
            if let Some(service) = services
                .get_opt(name)
                .await
                .context("Failed to inspect K8s Service for cleanup")?
                && self.owns_labels(service.metadata.labels.as_ref(), true)
            {
                let params = DeleteParams {
                    preconditions: Some(kube::api::Preconditions {
                        uid: service.metadata.uid,
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                ignore_not_found(services.delete(name, &params).await)
                    .context("Failed to delete K8s Service")?;
            }
        }
        Ok(())
    }

    /// Build the Pod spec for this sandbox
    fn build_pod_spec(&self, config: &SandboxConfig) -> Pod {
        let pod_name = Self::pod_name_for(&self.name);
        let mut labels = Self::pod_labels(&self.name);
        self.add_owner_label(&mut labels);

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

        // Explicit requests let the scheduler/autoscaler size a cold node correctly.
        let resource_requests = resource_limits.clone();

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
            volume_mounts: (!self.persistent_volume_claims.is_empty()).then(|| {
                self.persistent_volume_claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| k8s_openapi::api::core::v1::VolumeMount {
                        name: format!("workspace-{index}"),
                        mount_path: claim.mount_path.clone(),
                        read_only: Some(claim.read_only),
                        ..Default::default()
                    })
                    .collect()
            }),
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
            security_context: self.fs_group.map(|group| {
                k8s_openapi::api::core::v1::PodSecurityContext {
                    fs_group: Some(group),
                    ..Default::default()
                }
            }),
            volumes: (!self.persistent_volume_claims.is_empty()).then(|| {
                self.persistent_volume_claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| k8s_openapi::api::core::v1::Volume {
                        name: format!("workspace-{index}"),
                        persistent_volume_claim: Some(
                            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                claim_name: claim.claim_name.clone(),
                                read_only: Some(claim.read_only),
                            },
                        ),
                        ..Default::default()
                    })
                    .collect()
            }),
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
        self.add_owner_label(&mut match_labels);
        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel/sandbox".to_string(), identity.sandbox_label);

        self.add_owner_label(&mut labels);
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
    async fn create_network_policy(&mut self, client: &Client, attempt: &str) -> Result<()> {
        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &self.namespace);
        let mut policy = self.build_network_policy();
        policy
            .metadata
            .annotations
            .get_or_insert_with(BTreeMap::new)
            .insert("agentkernel/start-attempt".into(), attempt.to_string());
        let name = policy.metadata.name.as_deref().expect("policy name");
        if let Err(error) = np_api.create(&PostParams::default(), &policy).await {
            // A confirmed conflict belongs to another attempt. Do not adopt it.
            if matches!(&error, kube::Error::Api(status) if status.code == 409) {
                return Err(error).context("Failed to create NetworkPolicy");
            }
            match np_api.get_opt(name).await {
                Ok(Some(created))
                    if created
                        .metadata
                        .annotations
                        .as_ref()
                        .and_then(|annotations| annotations.get("agentkernel/start-attempt"))
                        .map(String::as_str)
                        == Some(attempt) =>
                {
                    // Policy creation succeeded despite the response failure.
                    // No Pod was submitted: rollback only this policy, never a
                    // same-named existing Pod. Retain ownership until confirmed.
                    self.client = Some(client.clone());
                    self.network_policy_created = true;
                    self.running = true;
                    let params = DeleteParams {
                        preconditions: Some(kube::api::Preconditions {
                            uid: created.metadata.uid,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    ignore_not_found(np_api.delete(name, &params).await)
                        .with_context(|| format!("NetworkPolicy creation failed: {error}; rollback failed. Retry stop for sandbox '{}'", self.name))?;
                    self.network_policy_created = false;
                    self.running = false;
                }
                Err(reconcile) => {
                    self.client = Some(client.clone());
                    self.network_policy_created = true;
                    self.running = true;
                    bail!(
                        "NetworkPolicy creation outcome is uncertain: {error}; reconciliation failed: {reconcile}. Retry stop for sandbox '{}' before starting again",
                        self.name
                    );
                }
                _ => {}
            }
            return Err(error).context("Failed to create NetworkPolicy");
        }
        Ok(())
    }

    /// Delete the NetworkPolicy for this sandbox
    async fn delete_network_policy(&self, client: &Client) -> Result<()> {
        let np_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &self.namespace);
        let identity = self.identity();
        for name in identity.network_policy_names() {
            if let Some(policy) = np_api
                .get_opt(name)
                .await
                .context("Failed to inspect NetworkPolicy for cleanup")?
                && self.owns_network_policy(&policy)
            {
                let params = DeleteParams {
                    preconditions: Some(kube::api::Preconditions {
                        uid: policy.metadata.uid.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                ignore_not_found(np_api.delete(name, &params).await)
                    .context("Failed to delete NetworkPolicy")?;
            }
        }
        Ok(())
    }

    async fn start_with_client(&mut self, config: &SandboxConfig, client: Client) -> Result<()> {
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
        let mut pod = self.build_pod_spec(config);
        let attempt = uuid::Uuid::new_v4().to_string();
        pod.metadata
            .annotations
            .get_or_insert_with(BTreeMap::new)
            .insert("agentkernel/start-attempt".into(), attempt.clone());
        let pod_name = pod
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| Self::pod_name_for(&self.name));

        // Install deny-all before a workload can execute, not after Pod creation.
        self.network_disabled = !config.network;
        if !config.network {
            self.create_network_policy(&client, &attempt).await?;
            self.network_policy_created = true;
        }

        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        if let Err(error) = pods.create(&PostParams::default(), &pod).await {
            // A transport error may occur after creation. Reconcile only this
            // attempt; never roll back a pre-existing Pod after a name conflict.
            let conflict = matches!(&error, kube::Error::Api(status) if status.code == 409);
            if !conflict {
                match pods.get_opt(&pod_name).await {
                    Ok(Some(created))
                        if created
                            .metadata
                            .annotations
                            .as_ref()
                            .and_then(|annotations| {
                                annotations.get("agentkernel/start-attempt")
                            })
                            == Some(&attempt) =>
                    {
                        self.client = Some(client.clone());
                        self.pod_name = Some(pod_name.clone());
                        self.running = true;
                        return Err(self.rollback_start(error.into()).await);
                    }
                    Err(reconcile) => {
                        self.client = Some(client.clone());
                        self.pod_name = Some(pod_name.clone());
                        self.running = true;
                        return Err(anyhow::anyhow!(
                            "Pod creation outcome is uncertain: {error}; reconciliation failed: {reconcile}. Retry stop for sandbox '{}' before starting again",
                            self.name
                        ));
                    }
                    _ => {}
                }
            }
            if self.network_policy_created {
                if let Err(cleanup) = self.delete_network_policy(&client).await {
                    self.client = Some(client.clone());
                    self.running = true;
                    return Err(anyhow::anyhow!(
                        "Pod creation failed: {error}; NetworkPolicy cleanup also failed: {cleanup:#}. Retry stop for sandbox '{}'",
                        self.name
                    ));
                }
                self.network_policy_created = false;
            }
            return Err(error).context("Failed to create K8s pod");
        }
        self.client = Some(client.clone());
        self.pod_name = Some(pod_name.clone());
        // Pending pods consume provisioned capacity too. Keep manager ownership
        // if startup and rollback both fail, even before the Running phase.
        self.running = true;

        // Create the per-sandbox internal Service after the pod and policy. If
        // this fails, roll back every resource created by this start attempt.
        if !config.ports.is_empty() {
            let service = self.build_service(config);
            if let Err(error) = self.create_service(&client, &service).await {
                return Err(self.rollback_start(error).await);
            }
            self.service_created = true;
        }

        // Wait for the pod to be running
        if let Err(error) = self.wait_for_running(&client, &pod_name).await {
            return Err(self.rollback_start(error).await);
        }

        self.pod_name = Some(pod_name);
        self.client = Some(client);
        self.running = true;

        Ok(())
    }

    /// Wait for the pod to reach the Running phase.
    /// Uses exponential backoff: 50ms → 100ms → 200ms → 500ms (capped).
    async fn wait_for_running(&self, client: &Client, pod_name: &str) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        let mut delay_ms: u64 = 50;
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(self.startup_timeout_secs);
        let mut last_status = "no pod status received".to_string();

        loop {
            let pod = tokio::time::timeout_at(deadline, pods.get(pod_name))
                .await
                .with_context(|| {
                    format!(
                        "Timed out after {}s waiting for pod '{pod_name}': {last_status}",
                        self.startup_timeout_secs
                    )
                })??;
            last_status = pod
                .status
                .as_ref()
                .map(|status| format!("{status:?}"))
                .unwrap_or_else(|| "no pod status received".to_string());
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
                bail!(
                    "Timed out after {}s waiting for pod '{pod_name}': {last_status}",
                    self.startup_timeout_secs
                );
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
        self.validate_configuration()?;

        // Build K8s client
        let orch_config = self.client_config();
        let client = Self::build_client(&orch_config).await?;

        self.start_with_client(config, client).await
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_env(cmd, &[]).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        // Lazily initialize the client and resolve the canonical or legacy pod
        // (e.g., reconnecting to a running pod after an upgrade).
        if self.client.is_none() {
            let orch_config = self.client_config();
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
        // A failed lookup/delete is not a successful stop. Preserve state so
        // callers can retry rather than losing a still-billable sandbox.
        if self.client.is_none() {
            self.client = Some(Self::build_client(&self.client_config()).await?);
        }
        let client = self.client.clone().expect("client initialized");
        self.delete_owned_pods(&client).await?;
        self.delete_network_policy(&client).await?;
        self.network_policy_created = false;
        self.delete_service(&client).await?;
        self.service_created = false;
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

/// Missing resources are already cleaned up; every other API error is actionable.
fn ignore_not_found<T>(result: std::result::Result<T, kube::Error>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
        Err(error) => Err(error.into()),
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
    fn nitro_config_builds_isolated_pod_with_retained_storage() {
        let config: crate::config::Config =
            toml::from_str(include_str!("../../examples/aws-nitro/agentkernel.toml")).unwrap();
        let sandbox = KubernetesSandbox::new("nitro-demo", &config.orchestrator);
        sandbox.validate_configuration().unwrap();
        assert_eq!(sandbox.startup_timeout_secs, 600);
        assert_eq!(config.orchestrator.warm_pool_size, 0);
        let pod = sandbox.build_pod_spec(&SandboxConfig::default());
        let spec = pod.spec.unwrap();
        assert_eq!(spec.runtime_class_name.as_deref(), Some("isolated-sandbox"));
        assert_eq!(spec.automount_service_account_token, Some(false));
        assert!(spec.host_network.is_none());
        assert!(spec.host_users.is_none());
        let volume = &spec.volumes.as_ref().unwrap()[0];
        let claim = volume.persistent_volume_claim.as_ref().unwrap();
        assert_eq!(claim.claim_name, "nitro-demo-workspace");
        assert_eq!(claim.read_only, Some(false));
        let mount = &spec.containers[0].volume_mounts.as_ref().unwrap()[0];
        assert_eq!(mount.name, volume.name);
        assert_eq!(mount.mount_path, "/workspace");
        assert_eq!(spec.security_context.unwrap().fs_group, Some(1000));
        let resources = spec.containers[0].resources.as_ref().unwrap();
        assert_eq!(resources.requests, resources.limits);
    }

    #[test]
    fn nitro_requires_explicit_target_before_api_access() {
        for (kubeconfig, context) in [
            (None, None),
            (Some("/chosen/config"), None),
            (None, Some("eks")),
        ] {
            let config = OrchestratorConfig {
                runtime_class: Some("isolated-sandbox".into()),
                kubeconfig: kubeconfig.map(str::to_string),
                context: context.map(str::to_string),
                ..Default::default()
            };
            let error = KubernetesSandbox::new("nitro", &config)
                .validate_configuration()
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("explicit orchestrator.kubeconfig and context")
            );
        }
    }

    #[tokio::test]
    async fn failed_start_rollback_keeps_runtime_owned() {
        let mock = mock_api(|_, _, _| api_error(403)).await;
        let mut sandbox = KubernetesSandbox::new("pending", &OrchestratorConfig::default());
        sandbox.client = Some(mock.client.clone());
        sandbox.pod_name = Some("agentkernel-pending".into());
        sandbox.running = true;
        let error = sandbox
            .rollback_start(anyhow::anyhow!("cold start timeout"))
            .await;
        assert!(error.to_string().contains("rollback failed"));
        assert!(error.to_string().contains("Retry stop"));
        assert!(sandbox.running);
        assert!(sandbox.pod_name.is_some());
    }

    #[test]
    fn caller_config_cannot_override_manager_owner_identity() {
        let config: OrchestratorConfig = toml::from_str("kubernetes_owner_id = 'forged'").unwrap();
        assert!(config.kubernetes_owner_id.is_none());
        let config = OrchestratorConfig {
            kubernetes_owner_id: Some("internal".into()),
            ..Default::default()
        };
        assert!(
            !serde_json::to_string(&config)
                .unwrap()
                .contains("kubernetes_owner_id")
        );
    }

    #[test]
    fn old_kubernetes_config_defaults_are_compatible() {
        let config: OrchestratorConfig = toml::from_str("namespace = 'old-cluster'").unwrap();
        assert_eq!(config.startup_timeout_secs, 120);
        assert!(config.persistent_volume_claims.is_empty());
        let sandbox = KubernetesSandbox::new("legacy", &config);
        sandbox.validate_configuration().unwrap();
        let pod = sandbox.build_pod_spec(&SandboxConfig::default());
        let spec = pod.spec.unwrap();
        assert!(spec.volumes.is_none());
        assert!(spec.security_context.is_none());
        assert!(spec.runtime_class_name.is_none());
    }

    #[test]
    fn kubeconfig_context_and_node_selection_survive_construction() {
        let config = OrchestratorConfig {
            kubeconfig: Some("/chosen/config".into()),
            context: Some("chosen-cluster".into()),
            node_selector: HashMap::from([("team".into(), "sandbox".into())]),
            ..Default::default()
        };
        let sandbox = KubernetesSandbox::new("cluster", &config);
        assert_eq!(sandbox.client_config().kubeconfig, config.kubeconfig);
        assert_eq!(sandbox.client_config().context, config.context);
        assert_eq!(
            sandbox
                .build_pod_spec(&SandboxConfig::default())
                .spec
                .unwrap()
                .node_selector,
            Some(BTreeMap::from([("team".into(), "sandbox".into())]))
        );
    }

    #[test]
    fn pvc_settings_validate_before_api_access() {
        for name in ["", "../claim", "UPPER", "bad_name"] {
            let config = OrchestratorConfig {
                persistent_volume_claims: vec![KubernetesVolumeClaim {
                    claim_name: name.into(),
                    mount_path: "/workspace".into(),
                    read_only: false,
                }],
                ..Default::default()
            };
            assert!(
                KubernetesSandbox::new("bad", &config)
                    .validate_configuration()
                    .is_err()
            );
        }
        for path in [
            "relative",
            "/",
            "/workspace/../etc",
            "/workspace/",
            "/workspace//a",
            "/workspace/./a",
        ] {
            let config = OrchestratorConfig {
                persistent_volume_claims: vec![KubernetesVolumeClaim {
                    claim_name: "existing-claim".into(),
                    mount_path: path.into(),
                    read_only: true,
                }],
                ..Default::default()
            };
            assert!(
                KubernetesSandbox::new("bad", &config)
                    .validate_configuration()
                    .is_err()
            );
        }
        for timeout in [0, 86_401, u64::MAX] {
            let config = OrchestratorConfig {
                startup_timeout_secs: timeout,
                ..Default::default()
            };
            assert!(
                KubernetesSandbox::new("bad", &config)
                    .validate_configuration()
                    .is_err()
            );
        }
        let claim = KubernetesVolumeClaim {
            claim_name: "existing".into(),
            mount_path: "/workspace".into(),
            read_only: true,
        };
        let config = OrchestratorConfig {
            persistent_volume_claims: vec![claim.clone(), claim],
            ..Default::default()
        };
        assert!(
            KubernetesSandbox::new("bad", &config)
                .validate_configuration()
                .is_err()
        );
    }

    #[test]
    fn readonly_claim_is_readonly_in_both_volume_and_mount() {
        let config = OrchestratorConfig {
            persistent_volume_claims: vec![KubernetesVolumeClaim {
                claim_name: "read-only".into(),
                mount_path: "/data".into(),
                read_only: true,
            }],
            ..Default::default()
        };
        let spec = KubernetesSandbox::new("readonly", &config)
            .build_pod_spec(&SandboxConfig::default())
            .spec
            .unwrap();
        assert_eq!(
            spec.volumes.unwrap()[0]
                .persistent_volume_claim
                .as_ref()
                .unwrap()
                .read_only,
            Some(true)
        );
        assert_eq!(
            spec.containers[0].volume_mounts.as_ref().unwrap()[0].read_only,
            Some(true)
        );
    }

    struct MockApi {
        client: Client,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockApi {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn mock_api(
        handler: impl Fn(String, String, serde_json::Value) -> (u16, serde_json::Value)
        + Send
        + Sync
        + 'static,
    ) -> MockApi {
        use http_body_util::{BodyExt, Full};
        use hyper::{Response, service::service_fn};
        use hyper_util::rt::TokioIo;
        use std::{convert::Infallible, sync::Arc};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let service =
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let handler = handler.clone();
                            async move {
                                let method = request.method().to_string();
                                let path = request.uri().path().to_string();
                                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                                let body = serde_json::from_slice(&bytes)
                                    .unwrap_or(serde_json::Value::Null);
                                let (status, response) = handler(method, path, body);
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .header("content-type", "application/json")
                                        .body(Full::new(bytes::Bytes::from(response.to_string())))
                                        .unwrap(),
                                )
                            }
                        });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_from(KubeConfig::new(
            format!("http://{address}").parse().unwrap(),
        ))
        .unwrap();
        MockApi { client, task }
    }

    fn api_error(code: u16) -> (u16, serde_json::Value) {
        (
            code,
            serde_json::json!({"apiVersion":"v1", "kind":"Status", "status":"Failure", "message":"mock error", "reason":if code == 404 { "NotFound" } else { "Forbidden" }, "code":code}),
        )
    }

    #[tokio::test]
    async fn stop_reports_delete_failure_and_remains_retryable() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let phase = Arc::new(AtomicUsize::new(0));
        let server_phase = phase.clone();
        let mock = mock_api(move |method, path, body| {
            assert!(!path.contains("persistentvolumeclaims"), "stop must retain storage");
            if path.ends_with("/pods/agentkernel-retry") {
                if method == "DELETE" {
                    assert_eq!(body["preconditions"]["uid"], "pod-uid");
                    if server_phase.load(Ordering::SeqCst) == 0 { return api_error(403); }
                    server_phase.store(2, Ordering::SeqCst);
                    return (200, serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Success","code":200}));
                }
                if server_phase.load(Ordering::SeqCst) < 2 {
                    return (200, serde_json::json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"agentkernel-retry","uid":"pod-uid","labels":{"agentkernel/managed-by":"agentkernel","agentkernel/sandbox":"retry"}}}));
                }
            }
            api_error(404)
        }).await;
        let mut sandbox = KubernetesSandbox::new("retry", &OrchestratorConfig::default());
        sandbox.client = Some(mock.client.clone());
        sandbox.running = true;
        sandbox.pod_name = Some("agentkernel-retry".into());
        assert!(
            sandbox
                .stop()
                .await
                .unwrap_err()
                .to_string()
                .contains("delete K8s pod")
        );
        assert!(sandbox.running);
        assert!(sandbox.pod_name.is_some());
        phase.store(1, Ordering::SeqCst);
        sandbox.stop().await.unwrap();
        assert!(!sandbox.running);
        assert!(sandbox.pod_name.is_none());
        sandbox.stop().await.unwrap(); // Already absent is idempotent.
    }

    #[tokio::test]
    async fn stop_waits_for_pod_disappearance_after_delete_acknowledgement() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let deleted = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let server_deleted = deleted.clone();
        let server_polls = polls.clone();
        let mock = mock_api(move |method, path, _| {
            if path.ends_with("/pods/agentkernel-finalizer") {
                if method == "DELETE" {
                    server_deleted.store(true, Ordering::SeqCst);
                    return (200, serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Success","code":200}));
                }
                if !server_deleted.load(Ordering::SeqCst) || server_polls.fetch_add(1, Ordering::SeqCst) < 2 {
                    return (200, serde_json::json!({"metadata":{"name":"agentkernel-finalizer","uid":"pod-uid","labels":{"agentkernel/managed-by":"agentkernel","agentkernel/sandbox":"finalizer"}}}));
                }
            }
            api_error(404)
        }).await;
        let mut sandbox = KubernetesSandbox::new("finalizer", &OrchestratorConfig::default());
        sandbox.client = Some(mock.client.clone());
        sandbox.running = true;
        sandbox.stop().await.unwrap();
        assert!(deleted.load(Ordering::SeqCst));
        assert_eq!(polls.load(Ordering::SeqCst), 3);
        assert!(!sandbox.running);
    }

    #[tokio::test]
    async fn network_policy_is_created_before_sandbox_pod() {
        use std::sync::{Arc, Mutex};
        let calls = Arc::new(Mutex::new(Vec::new()));
        let server_calls = calls.clone();
        let mock = mock_api(move |method, path, body| {
            server_calls
                .lock()
                .unwrap()
                .push(format!("{method} {path}"));
            if method == "POST" {
                return (201, body);
            }
            if path.contains("/pods/") {
                return (
                    200,
                    serde_json::json!({"metadata":{},"status":{"phase":"Running"}}),
                );
            }
            (200, serde_json::json!({"metadata":{}}))
        })
        .await;
        let mut sandbox = KubernetesSandbox::new("ordered", &OrchestratorConfig::default());
        sandbox
            .start_with_client(
                &SandboxConfig {
                    network: false,
                    ..Default::default()
                },
                mock.client.clone(),
            )
            .await
            .unwrap();
        let calls = calls.lock().unwrap();
        let policy = calls
            .iter()
            .position(|call| call.starts_with("POST") && call.ends_with("/networkpolicies"))
            .unwrap();
        let pod = calls
            .iter()
            .position(|call| call.starts_with("POST") && call.ends_with("/pods"))
            .unwrap();
        assert!(policy < pod);
    }

    #[tokio::test]
    async fn uncertain_policy_creation_is_reconciled_without_creating_a_pod() {
        use std::sync::{Arc, Mutex};
        for deletion_denied in [false, true] {
            let created = Arc::new(Mutex::new(None::<serde_json::Value>));
            let server_created = created.clone();
            let mock = mock_api(move |method, path, mut body| {
                assert!(!path.contains("/pods"), "policy failure must precede Pod submission");
                if method == "POST" && path.ends_with("/networkpolicies") {
                    body["metadata"]["uid"] = "policy-uid".into();
                    *server_created.lock().unwrap() = Some(body);
                    return api_error(500);
                }
                if path.contains("/networkpolicies/") {
                    if method == "DELETE" {
                        assert_eq!(body["preconditions"]["uid"], "policy-uid");
                        if deletion_denied { return api_error(403); }
                        *server_created.lock().unwrap() = None;
                        return (200, serde_json::json!({"kind":"Status","apiVersion":"v1","status":"Success","code":200}));
                    }
                    return server_created.lock().unwrap().clone().map(|policy| (200, policy)).unwrap_or_else(|| api_error(404));
                }
                (200, serde_json::json!({"metadata":{}}))
            }).await;
            let mut sandbox =
                KubernetesSandbox::new("policy-uncertain", &OrchestratorConfig::default());
            let result = sandbox
                .start_with_client(
                    &SandboxConfig {
                        network: false,
                        ..Default::default()
                    },
                    mock.client.clone(),
                )
                .await;
            assert!(result.is_err());
            assert_eq!(sandbox.running, deletion_denied);
            assert_eq!(created.lock().unwrap().is_some(), deletion_denied);
            assert_eq!(sandbox.network_policy_created, deletion_denied);
        }
    }

    #[tokio::test]
    async fn create_conflict_never_deletes_preexisting_pod() {
        let mock = mock_api(|method, path, _| {
            assert_ne!(method, "DELETE");
            if path.ends_with("/pods") {
                return api_error(409);
            }
            (200, serde_json::json!({"metadata":{}}))
        })
        .await;
        let mut sandbox = KubernetesSandbox::new("conflict", &OrchestratorConfig::default());
        assert!(
            sandbox
                .start_with_client(&SandboxConfig::default(), mock.client.clone())
                .await
                .is_err()
        );
        assert!(!sandbox.running);
    }

    #[tokio::test]
    async fn uncertain_creation_reconciles_only_current_attempt_before_rollback() {
        use std::sync::{Arc, Mutex};
        let created = Arc::new(Mutex::new(None::<serde_json::Value>));
        let server_created = created.clone();
        let mock = mock_api(move |method, path, mut body| {
            if method == "POST" && path.ends_with("/pods") {
                body["metadata"]["uid"] = "created-uid".into();
                *server_created.lock().unwrap() = Some(body);
                return api_error(500);
            }
            if path.contains("/pods/") {
                if method == "DELETE" {
                    assert_eq!(body["preconditions"]["uid"], "created-uid");
                    *server_created.lock().unwrap() = None;
                    return (200, serde_json::json!({"kind":"Status","apiVersion":"v1","status":"Success","code":200}));
                }
                return server_created.lock().unwrap().clone().map(|pod| (200, pod)).unwrap_or_else(|| api_error(404));
            }
            if path == "/api/v1/namespaces/agentkernel" { return (200, serde_json::json!({"metadata":{}})); }
            api_error(404)
        }).await;
        let mut sandbox = KubernetesSandbox::new("uncertain", &OrchestratorConfig::default());
        assert!(
            sandbox
                .start_with_client(&SandboxConfig::default(), mock.client.clone())
                .await
                .is_err()
        );
        assert!(created.lock().unwrap().is_none());
        assert!(!sandbox.running);
    }

    #[tokio::test]
    async fn manager_owner_binding_protects_same_named_resources() {
        let config = OrchestratorConfig {
            kubernetes_owner_id: Some("new-owner".into()),
            ..Default::default()
        };
        let mut sandbox = KubernetesSandbox::new("same-name", &config);
        for labels in [
            sandbox
                .build_pod_spec(&SandboxConfig::default())
                .metadata
                .labels,
            sandbox
                .build_service(&SandboxConfig::default())
                .metadata
                .labels,
            sandbox.build_network_policy().metadata.labels,
        ] {
            assert_eq!(
                labels.unwrap().get("agentkernel/owner").map(String::as_str),
                Some("new-owner")
            );
        }
        assert_eq!(
            sandbox
                .build_service(&SandboxConfig::default())
                .spec
                .unwrap()
                .selector
                .unwrap()
                .get("agentkernel/owner")
                .map(String::as_str),
            Some("new-owner")
        );
        assert_eq!(
            sandbox
                .build_network_policy()
                .spec
                .unwrap()
                .pod_selector
                .unwrap()
                .match_labels
                .unwrap()
                .get("agentkernel/owner")
                .map(String::as_str),
            Some("new-owner")
        );
        let mut old_pod = sandbox.build_pod_spec(&SandboxConfig::default());
        old_pod
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .insert("agentkernel/owner".into(), "old-owner".into());
        let mut old_policy = sandbox.build_network_policy();
        old_policy
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .insert("agentkernel/owner".into(), "old-owner".into());
        assert!(!sandbox.owns_labels(old_pod.metadata.labels.as_ref(), true));
        assert!(!sandbox.owns_network_policy(&old_policy));
        let mut old_service = sandbox.build_service(&SandboxConfig::default());
        old_service
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .insert("agentkernel/owner".into(), "old-owner".into());
        let mock = mock_api(move |method, path, _| {
            assert_eq!(
                method, "GET",
                "must not delete a different manager's resource"
            );
            if path.contains("/services/") {
                return (200, serde_json::to_value(&old_service).unwrap());
            }
            if path.contains("/networkpolicies/") {
                return (200, serde_json::to_value(&old_policy).unwrap());
            }
            (200, serde_json::to_value(&old_pod).unwrap())
        })
        .await;
        sandbox.client = Some(mock.client.clone());
        sandbox.stop().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_nitro_target_cannot_fall_back_during_cleanup() {
        let mut sandbox = KubernetesSandbox::new(
            "nitro",
            &OrchestratorConfig {
                runtime_class: Some("isolated-sandbox".into()),
                ..Default::default()
            },
        );
        let error = sandbox.stop().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit orchestrator.kubeconfig and context")
        );
        assert!(sandbox.client.is_none());
    }

    #[tokio::test]
    async fn stop_reports_lookup_and_auxiliary_cleanup_errors() {
        for blocked_resource in ["/pods/", "/networkpolicies/", "/services/"] {
            let mock = mock_api(move |method, path, _| {
                assert_eq!(method, "GET");
                if path.contains(blocked_resource) {
                    api_error(403)
                } else {
                    api_error(404)
                }
            })
            .await;
            let mut sandbox = KubernetesSandbox::new("blocked", &OrchestratorConfig::default());
            sandbox.client = Some(mock.client.clone());
            sandbox.running = true;
            assert!(sandbox.stop().await.is_err());
            assert!(sandbox.running);
        }
    }

    #[tokio::test]
    async fn stop_never_deletes_unowned_resources() {
        let mock = mock_api(|method, _, _| {
            assert_eq!(method, "GET");
            (
                200,
                serde_json::json!({"metadata":{"name":"agentkernel-unowned"}}),
            )
        })
        .await;
        let mut sandbox = KubernetesSandbox::new("unowned", &OrchestratorConfig::default());
        sandbox.client = Some(mock.client.clone());
        sandbox.stop().await.unwrap();
    }

    #[tokio::test]
    async fn cold_start_deadline_includes_pending_diagnostics() {
        let mock = mock_api(|_, _, _| (200, serde_json::json!({"apiVersion":"v1","kind":"Pod","metadata":{},"status":{"phase":"Pending","conditions":[{"type":"PodScheduled","status":"False","reason":"Unschedulable","message":"waiting for Nitro capacity"}]}}))).await;
        let sandbox = KubernetesSandbox::new(
            "cold",
            &OrchestratorConfig {
                startup_timeout_secs: 1,
                ..Default::default()
            },
        );
        let error = sandbox
            .wait_for_running(&mock.client, "agentkernel-cold")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("after 1s"));
        assert!(error.contains("Unschedulable"));
        assert!(error.contains("waiting for Nitro capacity"));
    }

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
