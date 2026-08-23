//! Configuration parsing for agentkernel.toml files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use crate::backend::FileInjection;
use crate::permissions::SecurityProfile;
use sha2::{Digest, Sha256};

/// LLM key configuration: maps API domain → vault key name.
///
/// Example:
/// ```toml
/// [llm_keys]
/// "api.openai.com" = "OPENAI_API_KEY"
/// "api.anthropic.com" = "ANTHROPIC_API_KEY"
/// ```
pub type LlmKeysConfig = std::collections::BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigFingerprint {
    modified: Option<SystemTime>,
    len: u64,
    content_hash: [u8; 32],
}

impl ConfigFingerprint {
    #[allow(dead_code)]
    fn for_path(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let content = std::fs::read(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
            content_hash: Sha256::digest(content).into(),
        })
    }

    fn from_metadata_and_content(metadata: &std::fs::Metadata, content: &[u8]) -> Self {
        Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
            content_hash: Sha256::digest(content).into(),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedConfig {
    fingerprint: ConfigFingerprint,
    config: Arc<Config>,
}

static CONFIG_CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedConfig>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const MAX_CACHED_CONFIGS: usize = 64;

/// Tenant-scoped LLM model allowlists.
///
/// ```toml
/// [llm_governance]
/// enabled = true
/// [llm_governance.tenants.acme]
/// openai = ["gpt-4o", "gpt-4o-mini"]
/// anthropic = ["claude-3-5-sonnet"]
/// ```
///
/// Tenant identity is resolved by the server from validated authentication
/// context and is never accepted from an LLM request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmGovernanceConfig {
    /// Enable fail-closed model allowlist enforcement in the LLM proxy.
    #[serde(default)]
    pub enabled: bool,
    /// Tenant ID → provider → allowed model IDs.
    #[serde(default)]
    pub tenants: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    >,
}
/// File entry for injecting files into the sandbox at startup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Source path on the host (relative to config file or absolute)
    pub source: String,
    /// Destination path inside the sandbox (must be absolute)
    pub dest: String,
    /// File mode (e.g., "0644") - optional, defaults to 0644
    #[serde(default = "default_file_mode")]
    pub mode: String,
}

fn default_file_mode() -> String {
    "0644".to_string()
}

/// Build configuration for custom Dockerfiles
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Path to Dockerfile (relative to config file or absolute)
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Build context directory (defaults to Dockerfile directory)
    #[serde(default)]
    pub context: Option<String>,
    /// Multi-stage build target (optional)
    #[serde(default)]
    pub target: Option<String>,
    /// Build arguments
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
    /// Disable build cache
    #[serde(default)]
    pub no_cache: bool,
}

/// Host-side Git integration for agent sandboxes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitConfig {
    /// Create a dedicated managed worktree when a host workspace is mounted.
    /// Disabled by default to preserve existing mount behavior.
    #[serde(default)]
    pub worktree: bool,
}

/// Configuration for Kubernetes/Nomad orchestration backends
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Orchestrator provider: "kubernetes" or "nomad"
    #[serde(default)]
    pub provider: Option<String>,
    /// Namespace for sandbox workloads (K8s namespace or Nomad namespace)
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Path to kubeconfig (optional, auto-detected from KUBECONFIG or ~/.kube/config)
    #[serde(default)]
    pub kubeconfig: Option<String>,
    /// Kubeconfig context to use
    #[serde(default)]
    pub context: Option<String>,
    /// Kubernetes runtime class (e.g., "gvisor", "kata") for stronger isolation
    #[serde(default)]
    pub runtime_class: Option<String>,
    /// Kubernetes service account for sandbox pods
    #[serde(default)]
    pub service_account: Option<String>,
    /// Node selector labels for pod scheduling
    #[serde(default)]
    pub node_selector: std::collections::HashMap<String, String>,
    /// Nomad server address (e.g., "http://127.0.0.1:4646")
    #[serde(default)]
    pub nomad_addr: Option<String>,
    /// Nomad ACL token (prefer NOMAD_TOKEN env var)
    #[serde(default)]
    pub nomad_token: Option<String>,
    /// Nomad task driver: "docker", "exec", "raw_exec"
    #[serde(default = "default_nomad_driver")]
    pub nomad_driver: String,
    /// Nomad datacenter
    #[serde(default)]
    pub nomad_datacenter: Option<String>,
    /// Number of pre-warmed sandbox instances in the pool
    #[serde(default = "default_warm_pool_size")]
    pub warm_pool_size: usize,
    /// Maximum number of concurrent sandboxes cluster-wide
    #[serde(default = "default_max_pool_size")]
    pub max_pool_size: usize,
    /// Container images to pre-warm in the pool
    #[serde(default)]
    pub warm_pool_images: Vec<String>,
    /// Hard cap on total concurrent sandboxes
    #[serde(default = "default_max_sandboxes")]
    pub max_sandboxes: usize,
}

fn default_namespace() -> String {
    "agentkernel".to_string()
}

fn default_nomad_driver() -> String {
    "docker".to_string()
}

fn default_warm_pool_size() -> usize {
    10
}

fn default_max_pool_size() -> usize {
    50
}

fn default_max_sandboxes() -> usize {
    200
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            provider: None,
            namespace: default_namespace(),
            kubeconfig: None,
            context: None,
            runtime_class: None,
            service_account: None,
            node_selector: std::collections::HashMap::new(),
            nomad_addr: None,
            nomad_token: None,
            nomad_driver: default_nomad_driver(),
            nomad_datacenter: None,
            warm_pool_size: default_warm_pool_size(),
            max_pool_size: default_max_pool_size(),
            warm_pool_images: Vec::new(),
            max_sandboxes: default_max_sandboxes(),
        }
    }
}

/// Shared configuration for hosted remote providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Default runtime profile for remote sandboxes.
    #[serde(default)]
    pub default_profile: Option<String>,
    /// Optional custom bridge executable or script path.
    #[serde(default)]
    pub bridge: Option<String>,
    /// Workspace sync mode (currently "managed").
    #[serde(default)]
    pub sync_mode: Option<String>,
    #[serde(default)]
    pub daytona: RemoteProviderConfig,
    #[serde(default)]
    pub runloop: RemoteProviderConfig,
    #[serde(default, rename = "e2b")]
    pub e2b: RemoteProviderConfig,
    #[serde(default)]
    pub modal: RemoteProviderConfig,
    #[serde(default, rename = "agentcomputer")]
    pub agentcomputer: RemoteProviderConfig,
    #[serde(default)]
    pub profiles: std::collections::BTreeMap<String, RemoteProfileConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteProviderConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub token_id: Option<String>,
    #[serde(default)]
    pub token_id_env: Option<String>,
    #[serde(default)]
    pub token_secret: Option<String>,
    #[serde(default)]
    pub token_secret_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteProfileConfig {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub bootstrap: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// TLS configuration for the `[api.tls]` section in agentkernel.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiTlsConfig {
    /// Enable TLS
    #[serde(default)]
    pub enabled: bool,
    /// Certificate PEM path
    #[serde(default)]
    pub cert: Option<String>,
    /// Private key PEM path
    #[serde(default)]
    pub key: Option<String>,
    /// Require TLS (no plain HTTP)
    #[serde(default)]
    pub require_tls: bool,
}

/// API server configuration for the `[api]` section in agentkernel.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiConfig {
    /// TLS configuration
    #[serde(default)]
    pub tls: ApiTlsConfig,
    /// Allow HTTP API callers to run commands as root (`sudo: true`).
    /// Disabled by default for least privilege.
    #[serde(default)]
    pub allow_sudo_exec: bool,
}

/// Workspace lifecycle scheduling configuration.
///
/// Scheduling is evaluated by the long-running API daemon.  All times are
/// measured from the sandbox's persisted `last_activity_at` timestamp and
/// cron expressions are evaluated in UTC using the standard five-field cron
/// format (minute, hour, day of month, month, day of week).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSchedulingConfig {
    /// Enable the scheduler loop.  It is enabled by default, but remains
    /// dormant until at least one policy is configured.
    #[serde(default = "default_scheduling_enabled")]
    pub enabled: bool,
    /// Stop running sandboxes after this many idle minutes.
    #[serde(
        default,
        alias = "autostop_minutes",
        alias = "auto_stop_after_minutes",
        alias = "auto_stop_minutes"
    )]
    pub autostop_after_minutes: Option<u64>,
    /// Start stopped, non-dormant sandboxes when this UTC cron expression
    /// matches.  A matching expression is a start trigger, not a continuous
    /// running window.
    #[serde(
        default,
        alias = "autostart_schedule",
        alias = "auto_start_schedule",
        alias = "auto_start_cron"
    )]
    pub autostart_cron: Option<String>,
    /// Mark stopped sandboxes dormant after this many unused days.
    #[serde(default, alias = "dormant_days", alias = "mark_dormant_after_days")]
    pub dormant_after_days: Option<u64>,
    /// Remove dormant sandboxes after this many days in the dormant state.
    #[serde(
        default,
        alias = "delete_dormant_after_days",
        alias = "dormant_cleanup_after_days",
        alias = "remove_dormant_days"
    )]
    pub remove_dormant_after_days: Option<u64>,
    /// Poll interval for the daemon enforcement loop.
    #[serde(default = "default_scheduling_interval_seconds", alias = "interval")]
    pub check_interval_seconds: u64,
}

/// A daemon-integrated user job schedule.
///
/// Schedules are deliberately separate from [`WorkspaceSchedulingConfig`]:
/// workspace scheduling manages infrastructure lifecycle, while these entries
/// run user work at a UTC cron boundary.  The target is tagged so a malformed
/// entry cannot accidentally run more than one kind of action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobScheduleConfig {
    /// Stable identifier used for API operations and persisted run state.
    pub id: String,
    /// Whether the daemon should consider this schedule.
    #[serde(default = "default_schedule_enabled")]
    pub enabled: bool,
    /// Five-field UTC cron expression.
    pub cron: String,
    /// Preferred target form: `target = { type = "...", ... }`.
    #[serde(default)]
    pub target: Option<JobScheduleTarget>,
    /// Flat target fields are accepted for simple TOML files and converted to
    /// the same tagged target during validation.
    #[serde(rename = "type", default)]
    pub target_type: Option<String>,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub orchestration: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub object_class: Option<String>,
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

fn default_schedule_enabled() -> bool {
    true
}

/// The one action a user schedule may execute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobScheduleTarget {
    SandboxCommand {
        sandbox: String,
        command: Vec<String>,
    },
    Orchestration {
        name: String,
        #[serde(default)]
        input: Option<serde_json::Value>,
    },
    ObjectMethod {
        class: String,
        object_id: String,
        method: String,
        #[serde(default)]
        args: Option<serde_json::Value>,
    },
}

impl JobScheduleConfig {
    /// Resolve either the tagged or flat TOML target and enforce exactly one
    /// target kind.  This is intentionally a fallible operation so all
    /// configuration errors can include the stable schedule id at startup.
    pub fn resolve_target(&self) -> anyhow::Result<JobScheduleTarget> {
        if let Some(target) = self.target.clone() {
            if self.target_type.is_some()
                || self.sandbox.is_some()
                || self.command.is_some()
                || self.orchestration.is_some()
                || self.object_class.is_some()
                || self.object_id.is_some()
                || self.method.is_some()
                || self.input.is_some()
                || self.args.is_some()
            {
                anyhow::bail!("target cannot be combined with flat target fields");
            }
            return Ok(target);
        }

        let kind = self.target_type.as_deref().unwrap_or_else(|| {
            if self.sandbox.is_some() || self.command.is_some() {
                "sandbox_command"
            } else if self.orchestration.is_some() {
                "orchestration"
            } else if self.object_class.is_some()
                || self.object_id.is_some()
                || self.method.is_some()
            {
                "object_method"
            } else {
                ""
            }
        });

        match kind {
            "sandbox_command" => Ok(JobScheduleTarget::SandboxCommand {
                sandbox: required_field(self.sandbox.as_deref(), "sandbox")?,
                command: required_command(self.command.as_deref())?,
            }),
            "orchestration" => Ok(JobScheduleTarget::Orchestration {
                name: required_field(self.orchestration.as_deref(), "orchestration")?,
                input: self.input.clone(),
            }),
            "object_method" => Ok(JobScheduleTarget::ObjectMethod {
                class: required_field(self.object_class.as_deref(), "object_class")?,
                object_id: required_field(self.object_id.as_deref(), "object_id")?,
                method: required_field(self.method.as_deref(), "method")?,
                args: self.args.clone(),
            }),
            "" => anyhow::bail!("exactly one target kind is required"),
            other => anyhow::bail!("unknown target type '{other}'"),
        }
    }
}

fn required_field(value: Option<&str>, name: &str) -> anyhow::Result<String> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        anyhow::bail!("{name} is required")
    }
    Ok(value.to_string())
}

fn required_command(value: Option<&[String]>) -> anyhow::Result<Vec<String>> {
    let command = value.unwrap_or_default();
    if command.is_empty() || command.iter().any(|part| part.trim().is_empty()) {
        anyhow::bail!("command must contain at least one non-empty argument")
    }
    Ok(command.to_vec())
}

fn default_scheduling_enabled() -> bool {
    true
}

fn default_scheduling_interval_seconds() -> u64 {
    60
}

impl Default for WorkspaceSchedulingConfig {
    fn default() -> Self {
        Self {
            enabled: default_scheduling_enabled(),
            autostop_after_minutes: None,
            autostart_cron: None,
            dormant_after_days: None,
            remove_dormant_after_days: None,
            check_interval_seconds: default_scheduling_interval_seconds(),
        }
    }
}

impl WorkspaceSchedulingConfig {
    /// Whether any lifecycle policy needs enforcement.
    pub fn has_policies(&self) -> bool {
        self.autostop_after_minutes.is_some()
            || self.autostart_cron.is_some()
            || self.dormant_after_days.is_some()
            || self.remove_dormant_after_days.is_some()
    }

    /// Whether the daemon should run the scheduler.
    pub fn is_active(&self) -> bool {
        self.enabled && self.has_policies()
    }
}

/// Trust anchor configuration for enterprise policy signing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustAnchorsConfig {
    /// Public key identifiers for Ed25519 signature verification
    #[serde(default)]
    pub keys: Vec<String>,
}

/// Explicit mapping from a provisioned SCIM group to local authorization.
///
/// Group IDs are used instead of display names so a rename cannot silently
/// change a user's authorization.  The tenant is part of the mapping to keep
/// an otherwise-valid group ID from being reused across tenants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScimGroupMapping {
    /// SCIM tenant ID this mapping applies to.
    pub tenant_id: String,
    /// Stable SCIM group ID to map. Exactly one of `group_id` and
    /// `group_external_id` must be configured.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Stable IdP-provided SCIM `externalId` to map. This is usually the
    /// practical choice because the server generates SCIM resource IDs.
    #[serde(default)]
    pub group_external_id: Option<String>,
    /// Cedar roles granted while a user is an active member of the group.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Optional tenant team ID materialized for Cedar team-based policies.
    #[serde(default)]
    pub team_id: Option<String>,
}

/// Enterprise policy management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterpriseConfig {
    /// Enable enterprise policy management
    #[serde(default)]
    pub enabled: bool,
    /// URL of the enterprise policy server
    #[serde(default)]
    pub policy_server: Option<String>,
    /// Path to a local Cedar policy file. Relative paths are resolved from
    /// the explicit server configuration file directory.
    #[serde(default)]
    pub policy_file: Option<String>,
    /// Organization identifier
    #[serde(default)]
    pub org_id: Option<String>,
    /// Environment variable name containing the API key
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Offline mode behavior: fail_closed, cached_with_expiry, cached_indefinite, default_policy
    #[serde(default = "default_offline_mode")]
    pub offline_mode: String,
    /// Maximum cache age in hours before requiring a refresh
    #[serde(default = "default_cache_max_age_hours")]
    pub cache_max_age_hours: u64,
    /// Trust anchors for policy bundle signature verification
    #[serde(default)]
    pub trust_anchors: TrustAnchorsConfig,
    /// Default roles for API-key authenticated users
    #[serde(default = "default_enterprise_roles")]
    pub default_roles: Vec<String>,
    /// JWKS URL for JWT validation (optional, enables JWT auth)
    #[serde(default)]
    pub jwks_url: Option<String>,
    /// Explicit, tenant-scoped SCIM group authorization mappings.
    #[serde(default)]
    pub scim_group_mappings: Vec<ScimGroupMapping>,
}

fn default_enterprise_roles() -> Vec<String> {
    vec!["developer".to_string()]
}

fn default_offline_mode() -> String {
    "cached_with_expiry".to_string()
}

fn default_cache_max_age_hours() -> u64 {
    24
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            policy_server: None,
            policy_file: None,
            org_id: None,
            api_key_env: None,
            offline_mode: default_offline_mode(),
            cache_max_age_hours: default_cache_max_age_hours(),
            trust_anchors: TrustAnchorsConfig::default(),
            default_roles: default_enterprise_roles(),
            jwks_url: None,
            scim_group_mappings: Vec::new(),
        }
    }
}

/// Transport security configuration for sandbox access
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportConfig {
    /// Enable SSH access for sandboxes
    #[serde(default)]
    pub ssh: bool,
    /// Vault address for SSH CA (default: $VAULT_ADDR)
    pub vault_addr: Option<String>,
    /// Vault SSH secrets engine mount (default: "ssh")
    #[serde(default = "default_ssh_mount")]
    pub vault_ssh_mount: String,
    /// Vault SSH role (default: "agentkernel-client")
    #[serde(default = "default_ssh_role")]
    pub vault_ssh_role: String,
    /// Certificate TTL (default: "30m")
    #[serde(default = "default_cert_ttl")]
    pub cert_ttl: String,
    /// Require encrypted transport for all port mappings
    #[serde(default)]
    pub require_encrypted: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            ssh: false,
            vault_addr: None,
            vault_ssh_mount: default_ssh_mount(),
            vault_ssh_role: default_ssh_role(),
            cert_ttl: default_cert_ttl(),
            require_encrypted: false,
        }
    }
}

fn default_ssh_mount() -> String {
    "ssh".to_string()
}

fn default_ssh_role() -> String {
    "agentkernel-client".to_string()
}

fn default_cert_ttl() -> String {
    "30m".to_string()
}

/// Root configuration structure matching agentkernel.toml schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub resources: ResourcesConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    /// Build configuration for custom Dockerfiles
    #[serde(default)]
    pub build: BuildConfig,
    /// Git worktree isolation configuration.
    #[serde(default)]
    pub git: GitConfig,
    /// Files to inject into the sandbox at startup
    #[serde(default, rename = "files")]
    pub files: Vec<FileEntry>,
    /// Orchestrator configuration for Kubernetes/Nomad backends
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// Remote provider configuration for hosted sandboxes
    #[serde(default)]
    pub remote: RemoteConfig,
    /// Enterprise policy management
    #[serde(default)]
    pub enterprise: EnterpriseConfig,
    /// API server configuration
    #[serde(default)]
    pub api: ApiConfig,
    /// Workspace lifecycle scheduling.  `workspace` is accepted as an alias
    /// for users who prefer grouping this policy under the workspace name.
    #[serde(default, alias = "workspace")]
    pub scheduling: WorkspaceSchedulingConfig,
    /// User job schedules evaluated by the daemon in UTC.
    #[serde(default, rename = "schedule")]
    pub schedules: Vec<JobScheduleConfig>,
    /// Proxy hooks configuration
    #[serde(default)]
    pub proxy: ProxyHooksConfig,
    /// Secret bindings: maps env var name → target host.
    /// On sandbox creation the host env var is read and a proxy secret binding
    /// is created automatically (format: `KEY=value:host`).
    #[serde(default)]
    pub secrets: std::collections::BTreeMap<String, String>,
    /// Org-level LLM API key mappings: domain → vault key name.
    /// Keys configured here are auto-injected via proxy for all sandboxes
    /// unless overridden by sandbox-specific secret bindings.
    #[serde(default)]
    pub llm_keys: LlmKeysConfig,
    /// Tenant-scoped LLM model governance policy.
    #[serde(default)]
    pub llm_governance: LlmGovernanceConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Security profile: permissive, moderate (default), restrictive
    #[serde(default)]
    pub profile: SecurityProfile,
    /// Allow network access (overrides profile)
    pub network: Option<bool>,
    /// Mount current directory (overrides profile)
    pub mount_cwd: Option<bool>,
    /// Network domain filtering rules
    #[serde(default)]
    pub domains: DomainConfig,
    /// Command/binary execution rules
    #[serde(default)]
    pub commands: CommandConfig,
    /// Seccomp profile name or path
    #[serde(default)]
    pub seccomp: Option<String>,
    /// Transport security (SSH access)
    #[serde(default)]
    pub transport: TransportConfig,
}

/// Domain filtering configuration for network access control
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DomainConfig {
    /// Domains that are always allowed (API endpoints, etc.)
    #[serde(default)]
    pub allow: Vec<String>,
    /// Domains that are always blocked (cloud metadata, etc.)
    #[serde(default)]
    pub block: Vec<String>,
    /// Block all domains except those in allow list
    #[serde(default)]
    pub allowlist_only: bool,
}

impl DomainConfig {
    /// Returns true if any domain rules are configured
    pub fn has_rules(&self) -> bool {
        !self.allow.is_empty() || !self.block.is_empty() || self.allowlist_only
    }

    /// Check if a domain is allowed
    pub fn is_allowed(&self, domain: &str) -> bool {
        // First check blocklist
        for pattern in &self.block {
            if Self::matches_pattern(domain, pattern) {
                return false;
            }
        }

        // If allowlist_only mode, must be in allow list
        if self.allowlist_only {
            return self.allow.iter().any(|p| Self::matches_pattern(domain, p));
        }

        // Otherwise allow by default
        true
    }

    /// Check if domain matches a pattern (supports * wildcard prefix)
    fn matches_pattern(domain: &str, pattern: &str) -> bool {
        if pattern.starts_with("*.") {
            let suffix = &pattern[1..]; // ".example.com"
            domain.ends_with(suffix) || domain == &pattern[2..]
        } else {
            domain == pattern
        }
    }
}

/// Command/binary execution restrictions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandConfig {
    /// Commands/binaries that are allowed (if allowlist_only is true)
    #[serde(default)]
    pub allow: Vec<String>,
    /// Commands/binaries that are explicitly blocked
    #[serde(default)]
    pub block: Vec<String>,
    /// Block all commands except those in allow list
    #[serde(default)]
    pub allowlist_only: bool,
}

impl CommandConfig {
    /// Check if a command is allowed
    pub fn is_allowed(&self, command: &str) -> bool {
        // Extract the binary name (first part of command)
        let binary = command.split_whitespace().next().unwrap_or(command);
        let binary_name = std::path::Path::new(binary)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(binary);

        // Check blocklist
        if self.block.iter().any(|b| b == binary_name || b == binary) {
            return false;
        }

        // If allowlist_only mode, must be in allow list
        if self.allowlist_only {
            return self.allow.iter().any(|a| a == binary_name || a == binary);
        }

        // Otherwise allow by default
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub name: String,
    /// Runtime shorthand: base, python, node, go, rust, ruby, java, c, dotnet
    #[serde(default = "default_runtime")]
    pub runtime: String,
    /// Custom Docker image (overrides runtime if specified)
    #[serde(default)]
    pub base_image: Option<String>,
    /// Shell script to run inside the sandbox after start (e.g., install CLIs)
    #[serde(default)]
    pub init_script: Option<String>,
}

fn default_runtime() -> String {
    "base".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Preferred AI agent: claude, gemini, codex, opencode
    #[serde(default = "default_agent")]
    pub preferred: String,
    /// Compatibility mode: native, claude, codex, gemini
    /// Sets agent-specific permissions and network policies
    #[serde(default)]
    pub compatibility_mode: Option<String>,
    /// Git author/committer name exposed inside the sandbox.
    #[serde(default)]
    pub git_name: Option<String>,
    /// Git author/committer email exposed inside the sandbox.
    #[serde(default)]
    pub git_email: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            preferred: default_agent(),
            compatibility_mode: None,
            git_name: None,
            git_email: None,
        }
    }
}

impl AgentConfig {
    /// Return Git's process-scoped configuration environment for the configured
    /// agent identity. This keeps agent commits distinct without writing to a
    /// mounted user's global Git configuration.
    pub fn git_config_env(&self) -> Vec<(String, String)> {
        let (Some(name), Some(email)) = (&self.git_name, &self.git_email) else {
            return Vec::new();
        };
        if name.trim().is_empty() || email.trim().is_empty() {
            return Vec::new();
        }

        vec![
            ("GIT_CONFIG_COUNT".to_string(), "2".to_string()),
            ("GIT_CONFIG_KEY_0".to_string(), "user.name".to_string()),
            ("GIT_CONFIG_VALUE_0".to_string(), name.clone()),
            ("GIT_CONFIG_KEY_1".to_string(), "user.email".to_string()),
            ("GIT_CONFIG_VALUE_1".to_string(), email.clone()),
        ]
    }
}

fn default_agent() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesConfig {
    /// Number of vCPUs (default: 1)
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// Memory limit in MB (default: 512)
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u64,
}

/// Safe, portable subset of a sandbox configuration used by the export/import
/// commands and the desktop app.
///
/// Runtime state (UUIDs, backend identifiers, timestamps, and secret
/// bindings) is deliberately excluded so an exported file can be shared and
/// re-imported without leaking host-specific or credential-bearing data.
#[derive(Debug, Clone, Serialize)]
pub struct SandboxConfigExport {
    pub sandbox: SandboxConfigExportSection,
    pub resources: ResourcesConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<SandboxConfigExportAgent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxConfigExportNetwork>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SandboxConfigExportSection {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SandboxConfigExportAgent {
    pub preferred: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SandboxConfigExportNetwork {
    pub ports: Vec<String>,
}

impl SandboxConfigExport {
    /// Build an export from persisted sandbox settings without including
    /// runtime identity or secret material.
    pub fn from_parts(
        name: &str,
        image: &str,
        init_script: Option<&str>,
        vcpus: u32,
        memory_mb: u64,
        agent: Option<&str>,
        ports: Vec<String>,
    ) -> Self {
        Self {
            sandbox: SandboxConfigExportSection {
                name: name.to_string(),
                base_image: Some(image.to_string()),
                init_script: init_script.map(str::to_string),
            },
            resources: ResourcesConfig { vcpus, memory_mb },
            agent: agent.map(|preferred| SandboxConfigExportAgent {
                preferred: preferred.to_string(),
            }),
            network: (!ports.is_empty()).then_some(SandboxConfigExportNetwork { ports }),
        }
    }
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            vcpus: default_vcpus(),
            memory_mb: default_memory_mb(),
        }
    }
}

fn default_vcpus() -> u32 {
    1
}

fn default_memory_mb() -> u64 {
    512
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// vsock CID for host-guest communication (auto-assigned if not specified)
    pub vsock_cid: Option<u32>,
    /// Port mappings (Docker-style: "host:container", "container", "host:container/udp")
    #[serde(default)]
    pub ports: Vec<String>,
}

impl NetworkConfig {
    /// Parse port strings into PortMapping structs
    pub fn port_mappings(&self) -> anyhow::Result<Vec<crate::backend::PortMapping>> {
        self.ports
            .iter()
            .map(|s| crate::backend::PortMapping::parse(s))
            .collect()
    }
}

/// Proxy hooks configuration.
///
/// ```toml
/// [[proxy.hooks]]
/// name = "audit-to-file"
/// event = "on_request"
/// [proxy.hooks.target]
/// type = "file"
/// path = "/var/log/agentkernel/proxy.jsonl"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyHooksConfig {
    /// Hooks to register on proxy startup.
    #[serde(default)]
    pub hooks: Vec<ProxyHookEntry>,
}

/// A single proxy hook entry in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyHookEntry {
    pub name: String,
    pub event: String,
    pub target: ProxyHookTargetEntry,
}

/// Target for a proxy hook in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProxyHookTargetEntry {
    Webhook { url: String },
    File { path: String },
    Audit,
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        Self::from_str(&content)
    }

    /// Load a configuration file through the process-local parsed-config cache.
    ///
    /// Commands commonly load the same `agentkernel.toml` more than once while
    /// resolving an image, permissions, and file injections.  Keep those
    /// repeated reads cheap without making a long-lived server blind to edits:
    /// the cache entry is refreshed whenever the file's metadata or content
    /// changes.  A cloned value is returned so callers retain the ownership
    /// and mutation semantics of [`Self::from_file`].
    pub fn from_file_cached(path: &Path) -> Result<Self> {
        let cache_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let content = std::fs::read(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let fingerprint = ConfigFingerprint::from_metadata_and_content(&metadata, &content);

        if let Some(entry) = CONFIG_CACHE
            .lock()
            .expect("config cache lock poisoned")
            .get(&cache_path)
            && entry.fingerprint == fingerprint
        {
            return Ok((*entry.config).clone());
        }

        let content = String::from_utf8(content)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config = Arc::new(Self::from_str(&content)?);

        let mut cache = CONFIG_CACHE.lock().expect("config cache lock poisoned");
        if cache.len() >= MAX_CACHED_CONFIGS
            && !cache.contains_key(&cache_path)
            && let Some(evicted) = cache.keys().next().cloned()
        {
            cache.remove(&evicted);
        }
        cache.insert(
            cache_path,
            CachedConfig {
                fingerprint,
                config: Arc::clone(&config),
            },
        );

        Ok((*config).clone())
    }

    /// Return a content-sensitive fingerprint for a config file.
    #[allow(dead_code)]
    pub(crate) fn file_fingerprint(path: &Path) -> Option<ConfigFingerprint> {
        ConfigFingerprint::for_path(path)
    }

    /// Parse configuration from a TOML string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> Result<Self> {
        toml::from_str(content).context("Failed to parse TOML configuration")
    }

    /// Create a minimal config with just a name and agent type.
    pub fn minimal(name: &str, agent: &str) -> Self {
        Self {
            sandbox: SandboxConfig {
                name: name.to_string(),
                runtime: default_runtime(),
                base_image: None,
                init_script: None,
            },
            agent: AgentConfig {
                preferred: agent.to_string(),
                compatibility_mode: None,
                git_name: None,
                git_email: None,
            },
            resources: ResourcesConfig::default(),
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
            build: BuildConfig::default(),
            git: GitConfig::default(),
            files: Vec::new(),
            orchestrator: OrchestratorConfig::default(),
            remote: RemoteConfig::default(),
            enterprise: EnterpriseConfig::default(),
            api: ApiConfig::default(),
            scheduling: WorkspaceSchedulingConfig::default(),
            schedules: Vec::new(),
            proxy: ProxyHooksConfig::default(),
            secrets: std::collections::BTreeMap::new(),
            llm_keys: LlmKeysConfig::default(),
            llm_governance: LlmGovernanceConfig::default(),
        }
    }

    /// Get the effective permissions based on config
    ///
    /// If a compatibility_mode is set in [agent], uses that profile's permissions.
    /// Otherwise falls back to the [security] profile with overrides.
    pub fn get_permissions(&self) -> crate::permissions::Permissions {
        // Check for compatibility mode first
        if let Some(ref mode_str) = self.agent.compatibility_mode
            && let Some(mode) = crate::permissions::CompatibilityMode::from_str(mode_str)
        {
            let mut perms = mode.profile().permissions;

            // Still apply explicit overrides from [security]
            if let Some(network) = self.security.network {
                perms.network = network;
            }
            if let Some(mount_cwd) = self.security.mount_cwd {
                perms.mount_cwd = mount_cwd;
            }

            return perms;
        }

        // Fall back to security profile
        let mut perms = self.security.profile.permissions();

        // Apply overrides
        if let Some(network) = self.security.network {
            perms.network = network;
        }
        if let Some(mount_cwd) = self.security.mount_cwd {
            perms.mount_cwd = mount_cwd;
        }

        perms
    }

    /// Get the agent profile if a compatibility mode is configured
    #[allow(dead_code)]
    pub fn get_agent_profile(&self) -> Option<crate::permissions::AgentProfile> {
        self.agent
            .compatibility_mode
            .as_ref()
            .and_then(|mode_str| crate::permissions::CompatibilityMode::from_str(mode_str))
            .map(|mode| mode.profile())
    }

    /// Validate configuration for consistency. Returns warnings about
    /// misconfigured or unsupported settings.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let perms = self.get_permissions();

        let git_name_configured = self
            .agent
            .git_name
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        let git_email_configured = self
            .agent
            .git_email
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        if git_name_configured != git_email_configured {
            warnings.push(
                "Agent Git identity requires both 'git_name' and 'git_email' in [agent]."
                    .to_string(),
            );
        }

        // Warn if ports configured but network is disabled
        if !self.network.ports.is_empty() && !perms.network {
            warnings.push(
                "Port mappings in [network] have no effect because network access is disabled."
                    .to_string(),
            );
        }

        // Validate port strings are parseable
        for port_str in &self.network.ports {
            if crate::backend::PortMapping::parse(port_str).is_err() {
                warnings.push(format!("Invalid port mapping '{}' in [network].", port_str));
            }
        }

        // Warn if domain rules configured but network is disabled
        if self.security.domains.has_rules() && !perms.network {
            warnings.push(
                "Domain filtering rules in [security.domains] have no effect \
                 because network access is disabled."
                    .to_string(),
            );
        }

        // Check for domains appearing in both allow and block lists
        for domain in &self.security.domains.allow {
            if !self.security.domains.is_allowed(domain) {
                warnings.push(format!(
                    "Domain '{}' is in the allow list but matched by the block list \
                     (block takes precedence).",
                    domain
                ));
            }
        }

        // Warn that domain filtering is not yet enforced at runtime
        if self.security.domains.has_rules() && perms.network {
            warnings.push(
                "Domain filtering rules are configured but runtime DNS enforcement \
                 is not yet implemented. Rules are recorded for future use."
                    .to_string(),
            );
        }

        // Warn if require_encrypted is set but port mappings exist without TLS
        if self.security.transport.require_encrypted && !self.network.ports.is_empty() {
            warnings.push(
                "Transport encryption is required but port mappings do not include \
                 TLS termination. Consider using --ssh or adding a TLS proxy."
                    .to_string(),
            );
        }

        // Warn if SSH is enabled but security profile is restrictive (no network)
        if self.security.transport.ssh && !perms.network {
            warnings.push(
                "SSH is enabled but the security profile is 'restrictive' (no network). \
                 SSH requires network access."
                    .to_string(),
            );
        }

        // Warn if any remote provider has an inline api_key (prefer api_key_env
        // to avoid committing secrets into agentkernel.toml).
        let remote_providers = [
            ("daytona", &self.remote.daytona),
            ("runloop", &self.remote.runloop),
            ("e2b", &self.remote.e2b),
            ("modal", &self.remote.modal),
            ("agentcomputer", &self.remote.agentcomputer),
        ];
        for (name, provider) in &remote_providers {
            if provider.api_key.is_some() {
                warnings.push(format!(
                    "Remote provider '[remote.{}]' has 'api_key' set directly in the config \
                     file. This risks committing secrets to version control. \
                     Use 'api_key_env' to reference an environment variable instead.",
                    name
                ));
            }
            if provider.token_id.is_some() || provider.token_secret.is_some() {
                warnings.push(format!(
                    "Remote provider '[remote.{}]' has Modal-style token fields set directly in the \
                     config file. This risks committing secrets to version control. \
                     Use 'token_id_env' and 'token_secret_env' instead.",
                    name
                ));
            }
        }

        warnings
    }

    /// Get the effective Docker image for this config
    pub fn docker_image(&self) -> String {
        // base_image takes precedence over runtime shorthand
        if let Some(ref image) = self.sandbox.base_image {
            return image.clone();
        }

        // Map runtime to default Docker image
        match self.sandbox.runtime.as_str() {
            "python" => "python:3.12-alpine".to_string(),
            "node" => "node:22-alpine".to_string(),
            "go" => "golang:1.23-alpine".to_string(),
            "rust" => "rust:1.85-alpine".to_string(),
            "ruby" => "ruby:3.3-alpine".to_string(),
            "java" => "eclipse-temurin:21-alpine".to_string(),
            "c" => "gcc:14-bookworm".to_string(),
            "dotnet" => "mcr.microsoft.com/dotnet/sdk:8.0".to_string(),
            _ => "alpine:3.24".to_string(),
        }
    }

    /// Get the Dockerfile path if one is configured or auto-detected
    ///
    /// Returns the resolved path relative to the given base directory.
    pub fn dockerfile_path(&self, base_dir: &Path) -> Option<std::path::PathBuf> {
        // Explicit dockerfile in config takes priority
        if let Some(ref dockerfile) = self.build.dockerfile {
            let path = if Path::new(dockerfile).is_absolute() {
                Path::new(dockerfile).to_path_buf()
            } else {
                base_dir.join(dockerfile)
            };
            if path.exists() {
                return Some(path);
            }
        }

        // Auto-detect Dockerfile in base directory
        crate::languages::detect_dockerfile(base_dir)
    }

    /// Get the build context directory
    ///
    /// Defaults to the Dockerfile's directory if not explicitly set.
    pub fn build_context(&self, base_dir: &Path, dockerfile_path: &Path) -> std::path::PathBuf {
        if let Some(ref context) = self.build.context {
            if Path::new(context).is_absolute() {
                Path::new(context).to_path_buf()
            } else {
                base_dir.join(context)
            }
        } else {
            // Default to Dockerfile's directory
            dockerfile_path.parent().unwrap_or(base_dir).to_path_buf()
        }
    }

    /// Check if this config requires building from a Dockerfile
    pub fn requires_build(&self, base_dir: &Path) -> bool {
        self.dockerfile_path(base_dir).is_some()
    }

    /// Load and resolve files from the [[files]] section
    ///
    /// Resolves source paths relative to the given base directory (usually config file dir)
    /// and reads file contents into FileInjection structs.
    pub fn load_files(&self, base_dir: &Path) -> Result<Vec<FileInjection>> {
        let mut injections = Vec::new();

        for file in &self.files {
            // Resolve source path relative to base_dir
            let source_path = if Path::new(&file.source).is_absolute() {
                Path::new(&file.source).to_path_buf()
            } else {
                base_dir.join(&file.source)
            };

            // Read file content
            let content = std::fs::read(&source_path).with_context(|| {
                format!(
                    "Failed to read file for injection: {}",
                    source_path.display()
                )
            })?;

            injections.push(FileInjection {
                content,
                dest: file.dest.clone(),
            });
        }

        Ok(injections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
            [sandbox]
            name = "test-app"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.sandbox.name, "test-app");
        assert_eq!(config.sandbox.runtime, "base");
        assert_eq!(config.agent.preferred, "claude");
        assert_eq!(config.resources.vcpus, 1);
        assert_eq!(config.resources.memory_mb, 512);
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            [sandbox]
            name = "python-app"
            runtime = "python"

            [agent]
            preferred = "gemini"

            [resources]
            vcpus = 2
            memory_mb = 1024

            [network]
            vsock_cid = 5
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.sandbox.name, "python-app");
        assert_eq!(config.sandbox.runtime, "python");
        assert_eq!(config.agent.preferred, "gemini");
        assert_eq!(config.resources.vcpus, 2);
        assert_eq!(config.resources.memory_mb, 1024);
        assert_eq!(config.network.vsock_cid, Some(5));
    }

    #[test]
    fn test_sandbox_config_export_is_importable_and_portable() {
        let export = SandboxConfigExport::from_parts(
            "shared-sandbox",
            "python:3.12-alpine",
            Some("echo ready > /workspace/ready"),
            2,
            1024,
            Some("codex"),
            vec!["8080:80".to_string()],
        );
        let content = toml::to_string_pretty(&export).unwrap();
        let parsed = Config::from_str(&content).unwrap();

        assert_eq!(parsed.sandbox.name, "shared-sandbox");
        assert_eq!(parsed.docker_image(), "python:3.12-alpine");
        assert_eq!(parsed.resources.vcpus, 2);
        assert_eq!(parsed.resources.memory_mb, 1024);
        assert_eq!(parsed.agent.preferred, "codex");
        assert_eq!(parsed.network.ports, vec!["8080:80"]);
        assert_eq!(
            parsed.sandbox.init_script.as_deref(),
            Some("echo ready > /workspace/ready")
        );
        assert!(!content.contains("secret"));
        assert!(!content.contains("uuid"));
    }

    #[test]
    fn test_parse_remote_config() {
        let toml = r#"
            [sandbox]
            name = "remote-app"

            [remote]
            default_profile = "node-dev"
            bridge = "./scripts/remote-bridge.mjs"
            sync_mode = "managed"

            [remote.daytona]
            api_key_env = "DAYTONA_API_KEY"
            organization = "acme"

            [remote.modal]
            token_id_env = "MODAL_TOKEN_ID"
            token_secret_env = "MODAL_TOKEN_SECRET"
            project = "agentkernel"

            [remote.profiles.node-dev]
            image = "node:22"
            workspace_dir = "/workspace"

            [remote.profiles.node-dev.env]
            NODE_ENV = "development"
        "#;

        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.remote.default_profile.as_deref(), Some("node-dev"));
        assert_eq!(
            config.remote.daytona.api_key_env.as_deref(),
            Some("DAYTONA_API_KEY")
        );
        assert_eq!(
            config.remote.modal.token_id_env.as_deref(),
            Some("MODAL_TOKEN_ID")
        );
        assert_eq!(
            config.remote.modal.token_secret_env.as_deref(),
            Some("MODAL_TOKEN_SECRET")
        );
        assert_eq!(config.remote.modal.project.as_deref(), Some("agentkernel"));
        let profile = config.remote.profiles.get("node-dev").unwrap();
        assert_eq!(profile.image.as_deref(), Some("node:22"));
        assert_eq!(profile.workspace_dir.as_deref(), Some("/workspace"));
        assert_eq!(
            profile.env.get("NODE_ENV").map(String::as_str),
            Some("development")
        );
    }

    #[test]
    fn test_parse_files_config() {
        let toml = r#"
            [sandbox]
            name = "test-app"

            [[files]]
            source = "./config.json"
            dest = "/app/config.json"

            [[files]]
            source = "./script.sh"
            dest = "/app/script.sh"
            mode = "0755"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.files.len(), 2);
        assert_eq!(config.files[0].source, "./config.json");
        assert_eq!(config.files[0].dest, "/app/config.json");
        assert_eq!(config.files[0].mode, "0644"); // default
        assert_eq!(config.files[1].source, "./script.sh");
        assert_eq!(config.files[1].dest, "/app/script.sh");
        assert_eq!(config.files[1].mode, "0755");
    }

    #[test]
    fn test_empty_files_config() {
        let toml = r#"
            [sandbox]
            name = "test-app"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.files.is_empty());
    }

    #[test]
    fn test_parse_build_config() {
        let toml = r#"
            [sandbox]
            name = "custom-app"

            [build]
            dockerfile = "./Dockerfile.dev"
            context = "./app"
            target = "runtime"
            no_cache = true

            [build.args]
            PYTHON_VERSION = "3.12"
            DEBUG = "true"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(
            config.build.dockerfile,
            Some("./Dockerfile.dev".to_string())
        );
        assert_eq!(config.build.context, Some("./app".to_string()));
        assert_eq!(config.build.target, Some("runtime".to_string()));
        assert!(config.build.no_cache);
        assert_eq!(
            config.build.args.get("PYTHON_VERSION"),
            Some(&"3.12".to_string())
        );
        assert_eq!(config.build.args.get("DEBUG"), Some(&"true".to_string()));
    }

    #[test]
    fn test_default_build_config() {
        let toml = r#"
            [sandbox]
            name = "test-app"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.build.dockerfile.is_none());
        assert!(config.build.context.is_none());
        assert!(config.build.target.is_none());
        assert!(!config.build.no_cache);
        assert!(config.build.args.is_empty());
    }

    #[test]
    fn test_agent_compatibility_mode() {
        let toml = r#"
            [sandbox]
            name = "claude-project"

            [agent]
            preferred = "claude"
            compatibility_mode = "claude"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.agent.preferred, "claude");
        assert_eq!(config.agent.compatibility_mode, Some("claude".to_string()));

        // Should get Claude-specific permissions
        let profile = config.get_agent_profile();
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert!(profile.permissions.mount_cwd); // Claude needs project access
        assert!(
            profile
                .network_policy
                .always_allow
                .contains(&"api.anthropic.com".to_string())
        );
    }

    #[test]
    fn test_agent_git_identity_builds_process_scoped_config() {
        let toml = r#"
            [sandbox]
            name = "codex-project"

            [agent]
            preferred = "codex"
            git_name = "Codex Agent"
            git_email = "codex@example.com"
        "#;
        let config = Config::from_str(toml).unwrap();

        assert_eq!(
            config.agent.git_config_env(),
            vec![
                ("GIT_CONFIG_COUNT".to_string(), "2".to_string()),
                ("GIT_CONFIG_KEY_0".to_string(), "user.name".to_string()),
                ("GIT_CONFIG_VALUE_0".to_string(), "Codex Agent".to_string()),
                ("GIT_CONFIG_KEY_1".to_string(), "user.email".to_string()),
                (
                    "GIT_CONFIG_VALUE_1".to_string(),
                    "codex@example.com".to_string(),
                ),
            ]
        );
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_agent_git_identity_requires_name_and_email() {
        let toml = r#"
            [sandbox]
            name = "codex-project"

            [agent]
            git_name = "Codex Agent"
        "#;
        let config = Config::from_str(toml).unwrap();

        assert!(config.agent.git_config_env().is_empty());
        assert!(
            config
                .validate()
                .iter()
                .any(|warning| warning.contains("requires both 'git_name' and 'git_email'"))
        );
    }

    #[test]
    fn test_agent_compatibility_mode_with_overrides() {
        let toml = r#"
            [sandbox]
            name = "claude-no-network"

            [agent]
            compatibility_mode = "claude"

            [security]
            network = false
        "#;
        let config = Config::from_str(toml).unwrap();

        // Should have Claude permissions but with network disabled
        let perms = config.get_permissions();
        assert!(perms.mount_cwd); // From Claude profile
        assert!(!perms.network); // Overridden by [security]
    }

    #[test]
    fn test_domain_config_allow() {
        let config = DomainConfig {
            allow: vec!["api.example.com".to_string(), "*.pypi.org".to_string()],
            block: vec!["169.254.169.254".to_string()],
            allowlist_only: false,
        };

        assert!(config.is_allowed("api.example.com"));
        assert!(config.is_allowed("pypi.org")); // Matches *.pypi.org
        assert!(config.is_allowed("files.pypi.org")); // Matches *.pypi.org
        assert!(config.is_allowed("random.com")); // Not blocked, not allowlist_only
        assert!(!config.is_allowed("169.254.169.254")); // Blocked
    }

    #[test]
    fn test_domain_config_allowlist_only() {
        let config = DomainConfig {
            allow: vec!["api.example.com".to_string(), "*.pypi.org".to_string()],
            block: vec![],
            allowlist_only: true,
        };

        assert!(config.is_allowed("api.example.com"));
        assert!(config.is_allowed("pypi.org"));
        assert!(!config.is_allowed("random.com")); // Not in allowlist
    }

    #[test]
    fn test_command_config_allow() {
        let config = CommandConfig {
            allow: vec!["python".to_string(), "node".to_string()],
            block: vec!["rm".to_string(), "sudo".to_string()],
            allowlist_only: false,
        };

        assert!(config.is_allowed("python script.py"));
        assert!(config.is_allowed("/usr/bin/python script.py"));
        assert!(config.is_allowed("echo hello")); // Not blocked
        assert!(!config.is_allowed("rm -rf /"));
        assert!(!config.is_allowed("sudo apt install"));
    }

    #[test]
    fn test_command_config_allowlist_only() {
        let config = CommandConfig {
            allow: vec!["python".to_string(), "node".to_string()],
            block: vec![],
            allowlist_only: true,
        };

        assert!(config.is_allowed("python"));
        assert!(config.is_allowed("node index.js"));
        assert!(!config.is_allowed("bash")); // Not in allowlist
    }

    #[test]
    fn test_security_config_with_domains() {
        let toml = r#"
            [sandbox]
            name = "restricted-app"

            [security]
            profile = "restrictive"

            [security.domains]
            allow = ["api.example.com", "*.pypi.org"]
            block = ["169.254.169.254"]
            allowlist_only = false
        "#;
        let config = Config::from_str(toml).unwrap();

        assert!(
            config
                .security
                .domains
                .allow
                .contains(&"api.example.com".to_string())
        );
        assert!(
            config
                .security
                .domains
                .block
                .contains(&"169.254.169.254".to_string())
        );
        assert!(!config.security.domains.allowlist_only);
    }

    #[test]
    fn test_security_config_with_commands() {
        let toml = r#"
            [sandbox]
            name = "restricted-app"

            [security]
            profile = "restrictive"

            [security.commands]
            allow = ["python", "node", "npm"]
            block = ["rm", "sudo", "chmod"]
            allowlist_only = true
        "#;
        let config = Config::from_str(toml).unwrap();

        assert!(
            config
                .security
                .commands
                .allow
                .contains(&"python".to_string())
        );
        assert!(config.security.commands.block.contains(&"sudo".to_string()));
        assert!(config.security.commands.allowlist_only);
    }

    #[test]
    fn test_security_config_with_seccomp() {
        let toml = r#"
            [sandbox]
            name = "hardened-app"

            [security]
            profile = "restrictive"
            seccomp = "default"
        "#;
        let config = Config::from_str(toml).unwrap();

        assert_eq!(config.security.seccomp, Some("default".to_string()));
    }

    #[test]
    fn test_domain_config_has_rules() {
        let empty = DomainConfig::default();
        assert!(!empty.has_rules());

        let with_allow = DomainConfig {
            allow: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(with_allow.has_rules());

        let allowlist_only = DomainConfig {
            allowlist_only: true,
            ..Default::default()
        };
        assert!(allowlist_only.has_rules());
    }

    #[test]
    fn test_validate_domain_rules_no_network() {
        let toml = r#"
            [sandbox]
            name = "test"

            [security]
            profile = "restrictive"

            [security.domains]
            allow = ["api.example.com"]
        "#;
        let config = Config::from_str(toml).unwrap();
        let warnings = config.validate();
        // restrictive profile has network=false, so domain rules are ineffective
        assert!(warnings.iter().any(|w| w.contains("no effect")));
    }

    #[test]
    fn test_validate_no_warnings_without_domain_rules() {
        let toml = r#"
            [sandbox]
            name = "test"
        "#;
        let config = Config::from_str(toml).unwrap();
        let warnings = config.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_enterprise_config_defaults() {
        let toml = r#"
            [sandbox]
            name = "test"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(!config.enterprise.enabled);
        assert!(config.enterprise.policy_server.is_none());
        assert!(config.enterprise.org_id.is_none());
        assert!(config.enterprise.api_key_env.is_none());
        assert_eq!(config.enterprise.offline_mode, "cached_with_expiry");
        assert_eq!(config.enterprise.cache_max_age_hours, 24);
        assert!(config.enterprise.trust_anchors.keys.is_empty());
    }

    #[test]
    fn test_cached_config_reloads_when_file_changes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("agentkernel.toml");
        let first_contents = "[sandbox]\nname = \"alpha\"\n";
        let updated_contents = "[sandbox]\nname = \"omega\"\n";
        assert_eq!(first_contents.len(), updated_contents.len());
        std::fs::write(&path, first_contents).unwrap();

        let first = Config::from_file_cached(&path).unwrap();
        let cached = Config::from_file_cached(&path).unwrap();
        assert_eq!(first.sandbox.name, "alpha");
        assert_eq!(cached.sandbox.name, "alpha");

        // Same-length rewrites must invalidate the cache even on filesystems
        // whose modification timestamps have coarse resolution.
        std::fs::write(&path, updated_contents).unwrap();
        let updated = Config::from_file_cached(&path).unwrap();
        assert_eq!(updated.sandbox.name, "omega");
    }

    #[test]
    fn test_enterprise_config_full() {
        let toml = r#"
            [sandbox]
            name = "enterprise-app"

            [enterprise]
            enabled = true
            policy_server = "https://policy.acme-corp.com"
            org_id = "acme-corp"
            api_key_env = "AGENTKERNEL_API_KEY"
            offline_mode = "fail_closed"
            cache_max_age_hours = 48

            [enterprise.trust_anchors]
            keys = ["key1-public", "key2-public"]
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.enterprise.enabled);
        assert_eq!(
            config.enterprise.policy_server,
            Some("https://policy.acme-corp.com".to_string())
        );
        assert_eq!(config.enterprise.org_id, Some("acme-corp".to_string()));
        assert_eq!(
            config.enterprise.api_key_env,
            Some("AGENTKERNEL_API_KEY".to_string())
        );
        assert_eq!(config.enterprise.offline_mode, "fail_closed");
        assert_eq!(config.enterprise.cache_max_age_hours, 48);
        assert_eq!(config.enterprise.trust_anchors.keys.len(), 2);
    }

    #[test]
    fn test_api_tls_config_defaults() {
        let toml = r#"
            [sandbox]
            name = "test"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(!config.api.tls.enabled);
        assert!(config.api.tls.cert.is_none());
        assert!(config.api.tls.key.is_none());
        assert!(!config.api.tls.require_tls);
        assert!(!config.api.allow_sudo_exec);
    }

    #[test]
    fn test_api_tls_config_full() {
        let toml = r#"
            [sandbox]
            name = "tls-app"

            [api]
            allow_sudo_exec = true

            [api.tls]
            enabled = true
            cert = "/etc/certs/api.pem"
            key = "/etc/certs/api-key.pem"
            require_tls = true
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.api.tls.enabled);
        assert_eq!(config.api.tls.cert, Some("/etc/certs/api.pem".to_string()));
        assert_eq!(
            config.api.tls.key,
            Some("/etc/certs/api-key.pem".to_string())
        );
        assert!(config.api.tls.require_tls);
        assert!(config.api.allow_sudo_exec);
    }

    #[test]
    fn test_api_tls_config_enabled_only() {
        let toml = r#"
            [sandbox]
            name = "self-signed-app"

            [api.tls]
            enabled = true
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.api.tls.enabled);
        assert!(config.api.tls.cert.is_none());
        assert!(config.api.tls.key.is_none());
        assert!(!config.api.tls.require_tls);
        assert!(!config.api.allow_sudo_exec);
    }

    #[test]
    fn test_transport_config_defaults() {
        let toml = r#"
            [sandbox]
            name = "test"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(!config.security.transport.ssh);
        assert!(config.security.transport.vault_addr.is_none());
        assert_eq!(config.security.transport.vault_ssh_mount, "ssh");
        assert_eq!(
            config.security.transport.vault_ssh_role,
            "agentkernel-client"
        );
        assert_eq!(config.security.transport.cert_ttl, "30m");
    }

    #[test]
    fn test_transport_config_full() {
        let toml = r#"
            [sandbox]
            name = "ssh-app"

            [security.transport]
            ssh = true
            vault_addr = "https://vault.example.com"
            vault_ssh_mount = "ssh-client"
            vault_ssh_role = "my-role"
            cert_ttl = "1h"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.security.transport.ssh);
        assert_eq!(
            config.security.transport.vault_addr,
            Some("https://vault.example.com".to_string())
        );
        assert_eq!(config.security.transport.vault_ssh_mount, "ssh-client");
        assert_eq!(config.security.transport.vault_ssh_role, "my-role");
        assert_eq!(config.security.transport.cert_ttl, "1h");
    }

    #[test]
    fn test_transport_config_ssh_only() {
        let toml = r#"
            [sandbox]
            name = "ssh-only"

            [security.transport]
            ssh = true
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.security.transport.ssh);
        // Defaults for vault fields
        assert_eq!(config.security.transport.vault_ssh_mount, "ssh");
        assert_eq!(
            config.security.transport.vault_ssh_role,
            "agentkernel-client"
        );
        assert_eq!(config.security.transport.cert_ttl, "30m");
    }

    #[test]
    fn test_transport_config_require_encrypted() {
        let toml = r#"
            [sandbox]
            name = "secure-app"

            [security.transport]
            require_encrypted = true
            ssh = true
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.security.transport.require_encrypted);
        assert!(config.security.transport.ssh);
    }

    #[test]
    fn test_transport_config_require_encrypted_default() {
        let toml = r#"
            [sandbox]
            name = "test"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(!config.security.transport.require_encrypted);
    }

    #[test]
    fn test_validate_require_encrypted_with_ports() {
        let toml = r#"
            [sandbox]
            name = "test"

            [security]
            profile = "permissive"

            [security.transport]
            require_encrypted = true

            [network]
            ports = ["8080:80"]
        "#;
        let config = Config::from_str(toml).unwrap();
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("encryption")));
    }

    #[test]
    fn test_validate_ssh_restrictive_profile_warning() {
        let toml = r#"
            [sandbox]
            name = "test"

            [security]
            profile = "restrictive"

            [security.transport]
            ssh = true
        "#;
        let config = Config::from_str(toml).unwrap();
        let warnings = config.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("SSH") || w.contains("ssh"))
        );
    }

    #[test]
    fn test_parse_llm_keys_config() {
        let toml = r#"
            [sandbox]
            name = "test"

            [llm_keys]
            "api.openai.com" = "OPENAI_API_KEY"
            "api.anthropic.com" = "ANTHROPIC_API_KEY"
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.llm_keys.len(), 2);
        assert_eq!(
            config.llm_keys.get("api.openai.com"),
            Some(&"OPENAI_API_KEY".to_string())
        );
        assert_eq!(
            config.llm_keys.get("api.anthropic.com"),
            Some(&"ANTHROPIC_API_KEY".to_string())
        );
    }

    #[test]
    fn test_parse_llm_governance_config() {
        let toml = r#"
            [sandbox]
            name = "test"

            [llm_governance]
            enabled = true

            [llm_governance.tenants.acme]
            openai = ["gpt-4o", "gpt-4o-mini"]
        "#;
        let config = Config::from_str(toml).unwrap();
        assert!(config.llm_governance.enabled);
        assert_eq!(config.llm_governance.tenants["acme"]["openai"].len(), 2);
    }
}
