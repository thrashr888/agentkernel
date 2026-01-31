//! Kubernetes CRD types and operator controller for AgentSandbox resources.
//!
//! Defines the `AgentSandbox` and `AgentSandboxPool` Custom Resource Definitions
//! using kube-derive. The operator watches AgentSandbox CRs and reconciles them
//! by creating/deleting pods as needed, reporting status back to the CR.
//!
//! Compile with `--features kubernetes` to enable.

#![cfg(feature = "kubernetes")]

use anyhow::{Context, Result};
use futures::StreamExt;
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config as WatcherConfig;
use kube::{Client, CustomResource, Resource, ResourceExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

// ===== CRD: AgentSandbox =====

/// Spec for the AgentSandbox custom resource
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[kube(
    group = "agentkernel.io",
    version = "v1alpha1",
    kind = "AgentSandbox",
    plural = "agentsandboxes",
    shortname = "asb",
    status = "AgentSandboxStatus",
    namespaced
)]
pub struct AgentSandboxSpec {
    /// Container image to run
    pub image: String,
    /// Number of vCPUs (maps to K8s CPU limit in millicores)
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// Memory in MB
    #[serde(default = "default_memory")]
    pub memory_mb: u64,
    /// Whether to allow network access
    #[serde(default = "default_true")]
    pub network: bool,
    /// Whether the root filesystem should be read-only
    #[serde(default)]
    pub read_only: bool,
    /// Optional runtime class (e.g., "gvisor", "kata")
    #[serde(default)]
    pub runtime_class: Option<String>,
    /// Security profile: "permissive", "moderate", "restrictive"
    #[serde(default = "default_profile")]
    pub security_profile: String,
    /// Environment variables to set
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

fn default_vcpus() -> u32 {
    1
}
fn default_memory() -> u64 {
    512
}
fn default_true() -> bool {
    true
}
fn default_profile() -> String {
    "moderate".to_string()
}

/// Simple environment variable for the CRD spec
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Status reported by the operator on the AgentSandbox CR
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSandboxStatus {
    /// Current phase: Pending, Running, Stopped, Failed
    #[serde(default)]
    pub phase: String,
    /// Name of the managed pod
    #[serde(default)]
    pub pod_name: Option<String>,
    /// Reason for the current phase (on failure)
    #[serde(default)]
    pub message: Option<String>,
    /// When the sandbox was last reconciled
    #[serde(default)]
    pub last_reconciled: Option<String>,
}

// ===== CRD: AgentSandboxPool =====

/// Spec for the AgentSandboxPool custom resource
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[kube(
    group = "agentkernel.io",
    version = "v1alpha1",
    kind = "AgentSandboxPool",
    plural = "agentsandboxpools",
    shortname = "asp",
    status = "AgentSandboxPoolStatus",
    namespaced
)]
pub struct AgentSandboxPoolSpec {
    /// Target number of warm pods
    #[serde(default = "default_warm_size")]
    pub warm_pool_size: usize,
    /// Maximum number of total pods
    #[serde(default = "default_max_size")]
    pub max_pool_size: usize,
    /// Container image for pooled pods
    pub image: String,
    /// vCPUs per pod
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// Memory per pod in MB
    #[serde(default = "default_memory")]
    pub memory_mb: u64,
    /// Optional runtime class
    #[serde(default)]
    pub runtime_class: Option<String>,
}

fn default_warm_size() -> usize {
    10
}
fn default_max_size() -> usize {
    50
}

/// Status for the AgentSandboxPool CR
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSandboxPoolStatus {
    /// Current number of warm pods
    #[serde(default)]
    pub warm_pods: usize,
    /// Current number of active pods
    #[serde(default)]
    pub active_pods: usize,
    /// Total pods
    #[serde(default)]
    pub total_pods: usize,
    /// Last reconcile time
    #[serde(default)]
    pub last_reconciled: Option<String>,
}

// ===== Operator controller =====

/// Shared state for the operator reconciler
struct OperatorContext {
    client: Client,
}

/// Error type for the reconciler (wraps anyhow)
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
struct ReconcileError {
    #[from]
    source: anyhow::Error,
}

/// Reconcile an AgentSandbox CR.
///
/// Creates or deletes pods to match the desired state.
async fn reconcile_sandbox(
    sandbox: Arc<AgentSandbox>,
    ctx: Arc<OperatorContext>,
) -> std::result::Result<Action, ReconcileError> {
    let client = &ctx.client;
    let namespace = sandbox
        .namespace()
        .unwrap_or_else(|| "default".to_string());
    let name = sandbox.name_any();

    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let sandboxes: Api<AgentSandbox> = Api::namespaced(client.clone(), &namespace);

    let pod_name = format!("agentkernel-{}", name);

    // Check if the pod exists
    let existing_pod = pods.get_opt(&pod_name).await.ok().flatten();

    match existing_pod {
        Some(pod) => {
            // Pod exists -- update CR status from pod status
            let phase = pod
                .status
                .as_ref()
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            let status = serde_json::json!({
                "status": {
                    "phase": phase,
                    "podName": pod_name,
                    "lastReconciled": chrono::Utc::now().to_rfc3339(),
                }
            });

            let _ = sandboxes
                .patch_status(
                    &name,
                    &PatchParams::default(),
                    &Patch::Merge(&status),
                )
                .await;
        }
        None => {
            // Pod doesn't exist -- create it
            let spec = &sandbox.spec;

            // Build resource limits
            let mut resource_limits = BTreeMap::new();
            resource_limits.insert(
                "memory".to_string(),
                k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                    "{}Mi",
                    spec.memory_mb
                )),
            );
            resource_limits.insert(
                "cpu".to_string(),
                k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                    "{}m",
                    spec.vcpus * 1000
                )),
            );

            // Security context
            let security_context = k8s_openapi::api::core::v1::SecurityContext {
                privileged: Some(false),
                allow_privilege_escalation: Some(false),
                read_only_root_filesystem: Some(spec.read_only),
                run_as_non_root: Some(true),
                run_as_user: Some(1000),
                capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                    drop: Some(vec!["ALL".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            };

            // Build env vars
            let env_vars: Vec<k8s_openapi::api::core::v1::EnvVar> = spec
                .env
                .iter()
                .map(|e| k8s_openapi::api::core::v1::EnvVar {
                    name: e.name.clone(),
                    value: Some(e.value.clone()),
                    ..Default::default()
                })
                .collect();

            let container = Container {
                name: "sandbox".to_string(),
                image: Some(spec.image.clone()),
                command: Some(vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "sleep infinity".to_string(),
                ]),
                security_context: Some(security_context),
                resources: Some(k8s_openapi::api::core::v1::ResourceRequirements {
                    limits: Some(resource_limits),
                    ..Default::default()
                }),
                env: if env_vars.is_empty() {
                    None
                } else {
                    Some(env_vars)
                },
                ..Default::default()
            };

            // Owner reference so pod gets cleaned up when CR is deleted
            let owner_ref = sandbox.controller_owner_ref(&()).unwrap();

            let pod = Pod {
                metadata: ObjectMeta {
                    name: Some(pod_name.clone()),
                    namespace: Some(namespace.clone()),
                    labels: Some({
                        let mut labels = BTreeMap::new();
                        labels.insert(
                            "agentkernel.io/sandbox".to_string(),
                            name.clone(),
                        );
                        labels.insert(
                            "agentkernel.io/managed-by".to_string(),
                            "agentkernel-operator".to_string(),
                        );
                        labels
                    }),
                    owner_references: Some(vec![owner_ref]),
                    ..Default::default()
                },
                spec: Some(PodSpec {
                    containers: vec![container],
                    restart_policy: Some("Never".to_string()),
                    automount_service_account_token: Some(false),
                    runtime_class_name: spec.runtime_class.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            };

            match pods.create(&PostParams::default(), &pod).await {
                Ok(_) => {
                    let status = serde_json::json!({
                        "status": {
                            "phase": "Pending",
                            "podName": pod_name,
                            "lastReconciled": chrono::Utc::now().to_rfc3339(),
                        }
                    });

                    let _ = sandboxes
                        .patch_status(
                            &name,
                            &PatchParams::default(),
                            &Patch::Merge(&status),
                        )
                        .await;
                }
                Err(e) => {
                    let status = serde_json::json!({
                        "status": {
                            "phase": "Failed",
                            "message": format!("Failed to create pod: {}", e),
                            "lastReconciled": chrono::Utc::now().to_rfc3339(),
                        }
                    });

                    let _ = sandboxes
                        .patch_status(
                            &name,
                            &PatchParams::default(),
                            &Patch::Merge(&status),
                        )
                        .await;
                }
            }
        }
    }

    // Requeue after 30 seconds
    Ok(Action::requeue(std::time::Duration::from_secs(30)))
}

/// Error handler for reconcile failures
fn reconcile_error(
    _sandbox: Arc<AgentSandbox>,
    error: &ReconcileError,
    _ctx: Arc<OperatorContext>,
) -> Action {
    eprintln!("Reconcile error: {}", error);
    Action::requeue(std::time::Duration::from_secs(60))
}

/// Run the operator controller.
///
/// This is a long-running task that watches AgentSandbox CRs and reconciles them.
/// Call this from your main function or spawn it as a tokio task.
pub async fn run_operator(client: Client) -> Result<()> {
    let sandboxes: Api<AgentSandbox> = Api::all(client.clone());
    let pods: Api<Pod> = Api::all(client.clone());

    let context = Arc::new(OperatorContext {
        client: client.clone(),
    });

    // Build the controller
    Controller::new(sandboxes, WatcherConfig::default())
        .owns(pods, WatcherConfig::default())
        .run(reconcile_sandbox, reconcile_error, context)
        .for_each(|result| async move {
            match result {
                Ok((_obj, action)) => {
                    eprintln!("Reconciled: requeue in {:?}", action);
                }
                Err(e) => {
                    eprintln!("Controller error: {}", e);
                }
            }
        })
        .await;

    Ok(())
}

/// Generate CRD manifests as YAML for installation.
///
/// Returns YAML strings for both AgentSandbox and AgentSandboxPool CRDs.
pub fn generate_crd_manifests() -> Result<(String, String)> {
    let sandbox_crd = serde_yaml::to_string(
        &AgentSandbox::crd(),
    )
    .context("Failed to serialize AgentSandbox CRD")?;

    let pool_crd = serde_yaml::to_string(
        &AgentSandboxPool::crd(),
    )
    .context("Failed to serialize AgentSandboxPool CRD")?;

    Ok((sandbox_crd, pool_crd))
}
