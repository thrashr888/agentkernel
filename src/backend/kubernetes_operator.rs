//! Kubernetes CRD types and operator controller for AgentSandbox resources.
//!
//! Defines the `AgentSandbox`, `AgentSandboxPool`, `AgentKernelPolicy`, and
//! `ClusterAgentKernelPolicy` Custom Resource Definitions using kube-derive.
//! The operator watches these CRs and reconciles them by creating/deleting pods,
//! aggregating Cedar policies, and enforcing authorization decisions.
//!
//! Compile with `--features kubernetes` to enable.
//! Policy CRDs require `--features enterprise` for Cedar evaluation.

use anyhow::{Context, Result};
use futures::StreamExt;
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::CustomResourceExt;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config as WatcherConfig;
use kube::{Client, CustomResource, Resource, ResourceExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[cfg(feature = "enterprise")]
use kube::api::ListParams;

#[cfg(feature = "enterprise")]
use tokio::sync::RwLock;

#[cfg(feature = "enterprise")]
use crate::policy::{
    Action as PolicyAction, CedarEngine, PolicyAuditLogger, PolicyDecisionLog, Principal,
    Resource as PolicyResource, validate_cedar_syntax,
};

// ===== CRD: AgentSandbox =====

/// Spec for the AgentSandbox custom resource
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[kube(
    group = "agentkernel",
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
    group = "agentkernel",
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

// ===== CRD: AgentKernelPolicy (namespaced) =====

/// Spec for the AgentKernelPolicy custom resource.
///
/// Namespaced Cedar policy that applies to sandboxes in the same namespace.
/// Managed via `kubectl apply` and GitOps-compatible.
#[cfg(feature = "enterprise")]
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[kube(
    group = "agentkernel",
    version = "v1alpha1",
    kind = "AgentKernelPolicy",
    plural = "agentkernelpolicies",
    shortname = "akp",
    status = "AgentKernelPolicyStatus",
    namespaced
)]
pub struct AgentKernelPolicySpec {
    /// Cedar policy text
    pub cedar: String,
    /// Priority (higher values take precedence)
    #[serde(default)]
    pub priority: i32,
    /// Human-readable description
    #[serde(default)]
    pub description: Option<String>,
}

// ===== CRD: ClusterAgentKernelPolicy (cluster-scoped) =====

/// Spec for the ClusterAgentKernelPolicy custom resource.
///
/// Cluster-scoped Cedar policy that applies to all sandboxes globally.
/// Higher priority policies take precedence. Forbid rules always win.
#[cfg(feature = "enterprise")]
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[kube(
    group = "agentkernel",
    version = "v1alpha1",
    kind = "ClusterAgentKernelPolicy",
    plural = "clusteragentkernelpolicies",
    shortname = "cakp",
    status = "AgentKernelPolicyStatus"
)]
pub struct ClusterAgentKernelPolicySpec {
    /// Cedar policy text
    pub cedar: String,
    /// Priority (higher values take precedence)
    #[serde(default)]
    pub priority: i32,
    /// Human-readable description
    #[serde(default)]
    pub description: Option<String>,
}

/// Shared status for policy CRDs.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentKernelPolicyStatus {
    /// Whether the Cedar syntax is valid
    #[serde(default)]
    pub valid: bool,
    /// Whether the policy is actively loaded in the engine
    #[serde(default)]
    pub active: bool,
    /// Status message (error details on invalid policies)
    #[serde(default)]
    pub message: Option<String>,
    /// When the policy was last applied to the engine
    #[serde(default)]
    pub last_applied: Option<String>,
    /// Observed generation for change detection
    #[serde(default)]
    pub observed_generation: Option<i64>,
}

// ===== Operator controller =====

/// Shared state for the operator reconciler
struct OperatorContext {
    client: Client,
    #[cfg(feature = "enterprise")]
    cedar_engine: Option<Arc<RwLock<CedarEngine>>>,
    #[cfg(feature = "enterprise")]
    audit_logger: Option<PolicyAuditLogger>,
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
    let namespace = sandbox.namespace().unwrap_or_else(|| "default".to_string());
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
                .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
                .await;
        }
        None => {
            // Enterprise policy enforcement: check Create action before making pod
            #[cfg(feature = "enterprise")]
            if let Some(ref engine) = ctx.cedar_engine {
                let principal = build_principal_from_sandbox(&sandbox);
                let resource = build_resource_from_sandbox(&sandbox);

                let eng = engine.read().await;
                let decision = eng.evaluate(&principal, PolicyAction::Create, &resource, None);
                drop(eng);

                // Audit log the decision
                if let Some(ref audit) = ctx.audit_logger {
                    let log_entry = PolicyDecisionLog::new(
                        &principal.id,
                        PolicyAction::Create,
                        &resource.name,
                        decision.decision,
                        decision.matched_policies.clone(),
                        decision.evaluation_time_us,
                        Some(principal.org_id.clone()),
                        Some(decision.reason.clone()),
                    );
                    if let Err(e) = audit.log_decision(&log_entry) {
                        eprintln!("[enterprise] Failed to write audit log: {}", e);
                    }
                }

                if !decision.is_permit() {
                    let status = serde_json::json!({
                        "status": {
                            "phase": "Failed",
                            "message": format!("Policy denied: {}", decision.reason),
                            "lastReconciled": chrono::Utc::now().to_rfc3339(),
                        }
                    });

                    let _ = sandboxes
                        .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
                        .await;

                    return Ok(Action::requeue(std::time::Duration::from_secs(300)));
                }
            }

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
                        labels.insert("agentkernel/sandbox".to_string(), name.clone());
                        labels.insert(
                            "agentkernel/managed-by".to_string(),
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
                        .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
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
                        .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
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

// ===== Enterprise policy helpers =====

/// Build a Principal from sandbox CR annotations.
#[cfg(feature = "enterprise")]
fn build_principal_from_sandbox(sandbox: &AgentSandbox) -> Principal {
    let annotations = sandbox
        .metadata
        .annotations
        .as_ref()
        .cloned()
        .unwrap_or_default();

    let roles_str = annotations
        .get("agentkernel/roles")
        .cloned()
        .unwrap_or_default();
    let roles: Vec<String> = if roles_str.is_empty() {
        vec!["developer".to_string()]
    } else {
        roles_str.split(',').map(|s| s.trim().to_string()).collect()
    };

    Principal {
        id: annotations
            .get("agentkernel/user-id")
            .cloned()
            .unwrap_or_else(|| "anonymous".to_string()),
        email: annotations
            .get("agentkernel/email")
            .cloned()
            .unwrap_or_else(|| "anonymous@unknown".to_string()),
        org_id: annotations
            .get("agentkernel/org-id")
            .cloned()
            .unwrap_or_else(|| "default".to_string()),
        roles,
        mfa_verified: annotations
            .get("agentkernel/mfa-verified")
            .map(|v| v == "true")
            .unwrap_or(false),
    }
}

/// Build a policy Resource from sandbox spec.
#[cfg(feature = "enterprise")]
fn build_resource_from_sandbox(sandbox: &AgentSandbox) -> PolicyResource {
    let annotations = sandbox
        .metadata
        .annotations
        .as_ref()
        .cloned()
        .unwrap_or_default();

    PolicyResource {
        name: sandbox.name_any(),
        agent_type: annotations
            .get("agentkernel/agent-type")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        runtime: annotations
            .get("agentkernel/runtime")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

// ===== Policy CRD reconcilers =====

/// Aggregate all policy CRs across the cluster and reload the Cedar engine.
///
/// Collects both namespaced `AgentKernelPolicy` and cluster-scoped
/// `ClusterAgentKernelPolicy` CRs, sorts by scope and priority,
/// concatenates their Cedar text, and hot-reloads the engine.
#[cfg(feature = "enterprise")]
async fn aggregate_and_reload_policies(ctx: &Arc<OperatorContext>) -> Result<()> {
    let engine = match ctx.cedar_engine {
        Some(ref e) => e,
        None => return Ok(()),
    };
    let client = &ctx.client;

    // Collect cluster-scoped policies
    let cluster_policies: Api<ClusterAgentKernelPolicy> = Api::all(client.clone());
    let cluster_list = cluster_policies
        .list(&ListParams::default())
        .await
        .context("Failed to list ClusterAgentKernelPolicy CRs")?;

    // Collect namespaced policies
    let ns_policies: Api<AgentKernelPolicy> = Api::all(client.clone());
    let ns_list = ns_policies
        .list(&ListParams::default())
        .await
        .context("Failed to list AgentKernelPolicy CRs")?;

    // Build (priority, scope_weight, source_label, cedar_text) tuples
    // scope_weight: 0 = cluster (applied first), 1 = namespaced
    let mut policy_fragments: Vec<(i32, u8, String, String)> = Vec::new();

    for cr in &cluster_list.items {
        if validate_cedar_syntax(&cr.spec.cedar).is_ok() {
            let label = format!("cluster/{}", cr.name_any());
            policy_fragments.push((cr.spec.priority, 0, label, cr.spec.cedar.clone()));
        }
    }

    for cr in &ns_list.items {
        if validate_cedar_syntax(&cr.spec.cedar).is_ok() {
            let ns = cr.namespace().unwrap_or_else(|| "default".to_string());
            let label = format!("{}/{}", ns, cr.name_any());
            policy_fragments.push((cr.spec.priority, 1, label, cr.spec.cedar.clone()));
        }
    }

    // Sort: cluster-scoped first, then by priority descending
    policy_fragments.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));

    // Concatenate with source comments
    let aggregated = if policy_fragments.is_empty() {
        // Fallback: permit authenticated users when no CRDs exist
        r#"permit(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
);"#
        .to_string()
    } else {
        policy_fragments
            .iter()
            .map(|(priority, _scope, label, cedar)| {
                format!("// Source: {} (priority: {})\n{}", label, priority, cedar)
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    // Hot-reload the engine
    {
        let mut eng = engine.write().await;
        eng.update_policies(&aggregated)
            .context("Failed to reload aggregated policies")?;
    }

    // Update status on all valid CRs
    let now = chrono::Utc::now().to_rfc3339();

    for cr in &cluster_list.items {
        let valid = validate_cedar_syntax(&cr.spec.cedar).is_ok();
        let status = serde_json::json!({
            "status": {
                "valid": valid,
                "active": valid,
                "message": if valid { None } else { Some("Invalid Cedar syntax") },
                "lastApplied": if valid { Some(&now) } else { None },
                "observedGeneration": cr.metadata.generation,
            }
        });

        let api: Api<ClusterAgentKernelPolicy> = Api::all(client.clone());
        let _ = api
            .patch_status(
                &cr.name_any(),
                &PatchParams::default(),
                &Patch::Merge(&status),
            )
            .await;
    }

    for cr in &ns_list.items {
        let valid = validate_cedar_syntax(&cr.spec.cedar).is_ok();
        let ns = cr.namespace().unwrap_or_else(|| "default".to_string());
        let status = serde_json::json!({
            "status": {
                "valid": valid,
                "active": valid,
                "message": if valid { None } else { Some("Invalid Cedar syntax") },
                "lastApplied": if valid { Some(&now) } else { None },
                "observedGeneration": cr.metadata.generation,
            }
        });

        let api: Api<AgentKernelPolicy> = Api::namespaced(client.clone(), &ns);
        let _ = api
            .patch_status(
                &cr.name_any(),
                &PatchParams::default(),
                &Patch::Merge(&status),
            )
            .await;
    }

    eprintln!(
        "[enterprise] Loaded {} policy fragment(s) from CRDs",
        policy_fragments.len()
    );

    Ok(())
}

/// Reconcile a namespaced AgentKernelPolicy CR.
#[cfg(feature = "enterprise")]
async fn reconcile_policy(
    policy: Arc<AgentKernelPolicy>,
    ctx: Arc<OperatorContext>,
) -> std::result::Result<Action, ReconcileError> {
    let name = policy.name_any();
    let ns = policy.namespace().unwrap_or_else(|| "default".to_string());

    // Validate Cedar syntax
    if let Err(e) = validate_cedar_syntax(&policy.spec.cedar) {
        eprintln!("[enterprise] Invalid Cedar in {}/{}: {}", ns, name, e);

        let status = serde_json::json!({
            "status": {
                "valid": false,
                "active": false,
                "message": format!("Invalid Cedar syntax: {}", e),
                "observedGeneration": policy.metadata.generation,
            }
        });

        let api: Api<AgentKernelPolicy> = Api::namespaced(ctx.client.clone(), &ns);
        let _ = api
            .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
            .await;

        return Ok(Action::requeue(std::time::Duration::from_secs(30)));
    }

    // Valid -- aggregate all policies and reload
    if let Err(e) = aggregate_and_reload_policies(&ctx).await {
        eprintln!("[enterprise] Failed to reload policies: {}", e);
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

/// Reconcile a cluster-scoped ClusterAgentKernelPolicy CR.
#[cfg(feature = "enterprise")]
async fn reconcile_cluster_policy(
    policy: Arc<ClusterAgentKernelPolicy>,
    ctx: Arc<OperatorContext>,
) -> std::result::Result<Action, ReconcileError> {
    let name = policy.name_any();

    // Validate Cedar syntax
    if let Err(e) = validate_cedar_syntax(&policy.spec.cedar) {
        eprintln!("[enterprise] Invalid Cedar in cluster/{}: {}", name, e);

        let status = serde_json::json!({
            "status": {
                "valid": false,
                "active": false,
                "message": format!("Invalid Cedar syntax: {}", e),
                "observedGeneration": policy.metadata.generation,
            }
        });

        let api: Api<ClusterAgentKernelPolicy> = Api::all(ctx.client.clone());
        let _ = api
            .patch_status(&name, &PatchParams::default(), &Patch::Merge(&status))
            .await;

        return Ok(Action::requeue(std::time::Duration::from_secs(30)));
    }

    // Valid -- aggregate all policies and reload
    if let Err(e) = aggregate_and_reload_policies(&ctx).await {
        eprintln!("[enterprise] Failed to reload policies: {}", e);
    }

    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}

/// Error handler for policy reconcile failures.
#[cfg(feature = "enterprise")]
fn reconcile_policy_error(
    _policy: Arc<AgentKernelPolicy>,
    error: &ReconcileError,
    _ctx: Arc<OperatorContext>,
) -> Action {
    eprintln!("[enterprise] Policy reconcile error: {}", error);
    Action::requeue(std::time::Duration::from_secs(60))
}

/// Error handler for cluster policy reconcile failures.
#[cfg(feature = "enterprise")]
fn reconcile_cluster_policy_error(
    _policy: Arc<ClusterAgentKernelPolicy>,
    error: &ReconcileError,
    _ctx: Arc<OperatorContext>,
) -> Action {
    eprintln!("[enterprise] Cluster policy reconcile error: {}", error);
    Action::requeue(std::time::Duration::from_secs(60))
}

/// Run the operator controller.
///
/// Watches AgentSandbox CRs and reconciles them. When the `enterprise` feature
/// is enabled, also watches AgentKernelPolicy and ClusterAgentKernelPolicy CRs
/// for Cedar policy management.
pub async fn run_operator(
    client: Client,
    #[cfg(feature = "enterprise")] cedar_engine: Option<Arc<RwLock<CedarEngine>>>,
    #[cfg(feature = "enterprise")] audit_logger: Option<PolicyAuditLogger>,
) -> Result<()> {
    let sandboxes: Api<AgentSandbox> = Api::all(client.clone());
    let pods: Api<Pod> = Api::all(client.clone());

    let context = Arc::new(OperatorContext {
        client: client.clone(),
        #[cfg(feature = "enterprise")]
        cedar_engine,
        #[cfg(feature = "enterprise")]
        audit_logger,
    });

    // Build sandbox controller
    let sandbox_controller = Controller::new(sandboxes, WatcherConfig::default())
        .owns(pods, WatcherConfig::default())
        .run(reconcile_sandbox, reconcile_error, context.clone())
        .for_each(|result| async move {
            match result {
                Ok((_obj, action)) => {
                    eprintln!("Reconciled sandbox: requeue in {:?}", action);
                }
                Err(e) => {
                    eprintln!("Sandbox controller error: {}", e);
                }
            }
        });

    // Enterprise: also run policy controllers
    #[cfg(feature = "enterprise")]
    {
        let ns_policies: Api<AgentKernelPolicy> = Api::all(client.clone());
        let cluster_policies: Api<ClusterAgentKernelPolicy> = Api::all(client.clone());

        let policy_controller = Controller::new(ns_policies, WatcherConfig::default())
            .run(reconcile_policy, reconcile_policy_error, context.clone())
            .for_each(|result| async move {
                match result {
                    Ok((_obj, action)) => {
                        eprintln!("Reconciled policy: requeue in {:?}", action);
                    }
                    Err(e) => {
                        eprintln!("Policy controller error: {}", e);
                    }
                }
            });

        let cluster_policy_controller = Controller::new(cluster_policies, WatcherConfig::default())
            .run(
                reconcile_cluster_policy,
                reconcile_cluster_policy_error,
                context.clone(),
            )
            .for_each(|result| async move {
                match result {
                    Ok((_obj, action)) => {
                        eprintln!("Reconciled cluster policy: requeue in {:?}", action);
                    }
                    Err(e) => {
                        eprintln!("Cluster policy controller error: {}", e);
                    }
                }
            });

        // Run all three controllers concurrently
        tokio::select! {
            _ = sandbox_controller => {},
            _ = policy_controller => {},
            _ = cluster_policy_controller => {},
        }
    }

    // Non-enterprise: just run sandbox controller
    #[cfg(not(feature = "enterprise"))]
    sandbox_controller.await;

    Ok(())
}

/// Generate CRD manifests as YAML for installation.
///
/// Returns YAML strings for all CRDs: AgentSandbox, AgentSandboxPool,
/// and (with enterprise feature) AgentKernelPolicy and ClusterAgentKernelPolicy.
pub fn generate_crd_manifests() -> Result<Vec<String>> {
    let mut crds = Vec::new();

    crds.push(
        serde_yaml::to_string(&AgentSandbox::crd())
            .context("Failed to serialize AgentSandbox CRD")?,
    );

    crds.push(
        serde_yaml::to_string(&AgentSandboxPool::crd())
            .context("Failed to serialize AgentSandboxPool CRD")?,
    );

    #[cfg(feature = "enterprise")]
    {
        crds.push(
            serde_yaml::to_string(&AgentKernelPolicy::crd())
                .context("Failed to serialize AgentKernelPolicy CRD")?,
        );

        crds.push(
            serde_yaml::to_string(&ClusterAgentKernelPolicy::crd())
                .context("Failed to serialize ClusterAgentKernelPolicy CRD")?,
        );
    }

    Ok(crds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_crd_schema() {
        let crd = AgentSandbox::crd();
        assert_eq!(
            crd.metadata.name,
            Some("agentsandboxes.agentkernel".to_string())
        );

        let yaml = serde_yaml::to_string(&crd).unwrap();
        assert!(yaml.contains("AgentSandbox"));
        assert!(yaml.contains("agentsandboxes"));
        assert!(yaml.contains("v1alpha1"));
    }

    #[test]
    fn test_pool_crd_schema() {
        let crd = AgentSandboxPool::crd();
        assert_eq!(
            crd.metadata.name,
            Some("agentsandboxpools.agentkernel".to_string())
        );
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_policy_crd_schema() {
        let crd = AgentKernelPolicy::crd();
        assert_eq!(
            crd.metadata.name,
            Some("agentkernelpolicies.agentkernel".to_string())
        );

        let yaml = serde_yaml::to_string(&crd).unwrap();
        assert!(yaml.contains("AgentKernelPolicy"));
        assert!(yaml.contains("agentkernelpolicies"));
        assert!(yaml.contains("v1alpha1"));
        // Namespaced CRD should have Namespaced scope
        assert!(yaml.contains("Namespaced"));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_cluster_policy_crd_schema() {
        let crd = ClusterAgentKernelPolicy::crd();
        assert_eq!(
            crd.metadata.name,
            Some("clusteragentkernelpolicies.agentkernel".to_string())
        );

        let yaml = serde_yaml::to_string(&crd).unwrap();
        assert!(yaml.contains("ClusterAgentKernelPolicy"));
        assert!(yaml.contains("clusteragentkernelpolicies"));
        // Cluster-scoped CRD should have Cluster scope
        assert!(yaml.contains("Cluster"));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_validate_cedar_syntax_valid() {
        let valid_cedar = r#"permit(
            principal is AgentKernel::User,
            action,
            resource is AgentKernel::Sandbox
        );"#;

        assert!(validate_cedar_syntax(valid_cedar).is_ok());
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_validate_cedar_syntax_invalid() {
        let invalid_cedar = "this is not valid cedar policy text!!!";
        assert!(validate_cedar_syntax(invalid_cedar).is_err());
    }

    #[test]
    fn test_generate_crd_manifests() {
        let crds = generate_crd_manifests().unwrap();

        // Should have at least sandbox + pool CRDs
        assert!(crds.len() >= 2);

        // Check sandbox CRD
        assert!(crds[0].contains("AgentSandbox"));
        // Check pool CRD
        assert!(crds[1].contains("AgentSandboxPool"));

        // Enterprise feature adds 2 more
        #[cfg(feature = "enterprise")]
        {
            assert_eq!(crds.len(), 4);
            assert!(crds[2].contains("AgentKernelPolicy"));
            assert!(crds[3].contains("ClusterAgentKernelPolicy"));
        }
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_policy_spec_defaults() {
        let spec: AgentKernelPolicySpec =
            serde_json::from_str(r#"{"cedar": "permit(principal, action, resource);"}"#).unwrap();

        assert_eq!(spec.cedar, "permit(principal, action, resource);");
        assert_eq!(spec.priority, 0);
        assert!(spec.description.is_none());
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_policy_spec_with_priority() {
        let spec: AgentKernelPolicySpec = serde_json::from_str(
            r#"{"cedar": "forbid(principal, action, resource);", "priority": 100, "description": "Block all"}"#,
        )
        .unwrap();

        assert_eq!(spec.priority, 100);
        assert_eq!(spec.description.as_deref(), Some("Block all"));
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_policy_status_defaults() {
        let status = AgentKernelPolicyStatus::default();
        assert!(!status.valid);
        assert!(!status.active);
        assert!(status.message.is_none());
        assert!(status.last_applied.is_none());
        assert!(status.observed_generation.is_none());
    }

    #[cfg(feature = "enterprise")]
    #[test]
    fn test_build_principal_from_annotations() {
        // Create a minimal AgentSandbox with annotations
        let sandbox = AgentSandbox::new(
            "test-sandbox",
            AgentSandboxSpec {
                image: "alpine:3.20".to_string(),
                vcpus: 1,
                memory_mb: 512,
                network: true,
                read_only: false,
                runtime_class: None,
                security_profile: "moderate".to_string(),
                env: vec![],
            },
        );

        // Without annotations, should get defaults
        let principal = build_principal_from_sandbox(&sandbox);
        assert_eq!(principal.id, "anonymous");
        assert_eq!(principal.email, "anonymous@unknown");
        assert_eq!(principal.org_id, "default");
        assert_eq!(principal.roles, vec!["developer"]);
        assert!(!principal.mfa_verified);
    }
}
