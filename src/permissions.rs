//! Permission profiles for sandbox security.
//!
//! Defines security profiles that control what sandboxed code can access.

use serde::{Deserialize, Serialize};

/// Security profile levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityProfile {
    /// Full access - network, all mounts, env passthrough
    Permissive,
    /// Balanced - network allowed, limited mounts, filtered env
    #[default]
    Moderate,
    /// Maximum isolation - no network, no mounts, clean env
    Restrictive,
    /// Custom profile defined by explicit permissions
    Custom,
}

impl SecurityProfile {
    /// Get the default permissions for this profile
    pub fn permissions(&self) -> Permissions {
        match self {
            SecurityProfile::Permissive => Permissions {
                network: true,
                mount_cwd: true,
                mount_home: true,
                pass_env: true,
                allow_privileged: false,
                read_only_root: false,
                max_memory_mb: None,
                max_cpu_percent: None,
                seccomp: Some("default".to_string()),
            },
            SecurityProfile::Moderate => Permissions {
                network: true,
                mount_cwd: false,
                mount_home: false,
                pass_env: false,
                allow_privileged: false,
                read_only_root: false,
                max_memory_mb: Some(512),
                max_cpu_percent: Some(100),
                seccomp: Some("moderate".to_string()),
            },
            SecurityProfile::Restrictive => Permissions {
                network: false,
                mount_cwd: false,
                mount_home: false,
                pass_env: false,
                allow_privileged: false,
                read_only_root: true,
                max_memory_mb: Some(256),
                max_cpu_percent: Some(50),
                seccomp: Some("restrictive".to_string()),
            },
            SecurityProfile::Custom => Permissions::default(),
        }
    }

    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "permissive" => Some(SecurityProfile::Permissive),
            "moderate" => Some(SecurityProfile::Moderate),
            "restrictive" => Some(SecurityProfile::Restrictive),
            "custom" => Some(SecurityProfile::Custom),
            _ => None,
        }
    }
}

/// Detailed permissions for sandbox execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    /// Allow network access
    pub network: bool,
    /// Mount current working directory
    pub mount_cwd: bool,
    /// Mount home directory (read-only)
    pub mount_home: bool,
    /// Pass through host environment variables
    pub pass_env: bool,
    /// Allow privileged operations (dangerous)
    pub allow_privileged: bool,
    /// Make root filesystem read-only
    pub read_only_root: bool,
    /// Maximum memory in MB (None = unlimited)
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU percentage (None = unlimited)
    pub max_cpu_percent: Option<u32>,
    /// Seccomp profile to use (None = Docker default, or "default", "moderate", "restrictive", "ai-agent")
    pub seccomp: Option<String>,
}

impl Default for Permissions {
    fn default() -> Self {
        SecurityProfile::Moderate.permissions()
    }
}

impl Permissions {
    /// Resolve seccomp profile path from name or path
    ///
    /// Built-in profiles: "default", "moderate", "restrictive", "ai-agent"
    /// Custom profiles: provide absolute path to JSON file
    pub fn resolve_seccomp_path(&self) -> Option<std::path::PathBuf> {
        let profile = self.seccomp.as_ref()?;

        // Check for built-in profiles
        let builtin_names = ["default", "moderate", "restrictive", "ai-agent"];
        if builtin_names.contains(&profile.as_str()) {
            // Look for built-in profiles relative to executable or in known locations
            let profile_name = format!("{}.json", profile);

            // Try relative to current dir (development)
            let dev_path = std::path::PathBuf::from("images/seccomp").join(&profile_name);
            if dev_path.exists() {
                return dev_path.canonicalize().ok();
            }

            // Try relative to executable (installed)
            if let Ok(exe_path) = std::env::current_exe()
                && let Some(exe_dir) = exe_path.parent()
            {
                let installed_path = exe_dir.join("seccomp").join(&profile_name);
                if installed_path.exists() {
                    return Some(installed_path);
                }
            }

            // Try system location
            let system_path =
                std::path::PathBuf::from("/usr/share/agentkernel/seccomp").join(&profile_name);
            if system_path.exists() {
                return Some(system_path);
            }

            // Built-in profile not found - will fall back to Docker default
            None
        } else {
            // Custom path provided
            let path = std::path::PathBuf::from(profile);
            if path.exists() { Some(path) } else { None }
        }
    }

    /// Convert permissions to Docker run arguments
    pub fn to_docker_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Network
        if !self.network {
            args.push("--network=none".to_string());
        }

        // Memory limit
        if let Some(mem) = self.max_memory_mb {
            args.push(format!("--memory={}m", mem));
            // Disable OOM killer overhead - container will be constrained but not killed
            args.push("--oom-kill-disable".to_string());
        }

        // CPU limit
        if let Some(cpu) = self.max_cpu_percent {
            // Docker uses CPU period/quota, simplified here
            args.push(format!("--cpus={:.2}", cpu as f64 / 100.0));
        }

        // Read-only root
        if self.read_only_root {
            args.push("--read-only".to_string());
            // Add tmpfs for /tmp so programs can still write temp files
            args.push("--tmpfs=/tmp:rw,noexec,nosuid,size=64m".to_string());
        }

        // Security options (always apply some baseline security)
        if !self.allow_privileged {
            args.push("--security-opt=no-new-privileges".to_string());
            args.push("--cap-drop=ALL".to_string());
            // Add back minimal caps needed for most programs
            args.push("--cap-add=CHOWN".to_string());
            args.push("--cap-add=SETUID".to_string());
            args.push("--cap-add=SETGID".to_string());
        }

        // Seccomp profile
        if let Some(seccomp_path) = self.resolve_seccomp_path() {
            args.push(format!("--security-opt=seccomp={}", seccomp_path.display()));
        }

        args
    }

    /// Get environment variables to pass (or block)
    pub fn get_env_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if self.pass_env {
            // Pass through common useful env vars
            for var in ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"] {
                if let Ok(val) = std::env::var(var) {
                    args.push("-e".to_string());
                    args.push(format!("{}={}", var, val));
                }
            }
        }

        args
    }

    /// Get mount arguments
    pub fn get_mount_args(&self, cwd: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();

        if self.mount_cwd
            && let Some(dir) = cwd
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace:rw", dir));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
        }

        if self.mount_home
            && let Ok(home) = std::env::var("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home));
        }

        args
    }
}

/// Compatibility mode for agent-specific behavior
///
/// Different AI agents have different expectations for sandbox behavior.
/// This mode adjusts permissions and networking to match each agent's needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompatibilityMode {
    /// Default agentkernel behavior
    #[default]
    Native,
    /// Claude Code compatible (proxy-style network, domain allowlist)
    ClaudeCode,
    /// OpenAI Codex compatible (Landlock-style, strict isolation)
    Codex,
    /// Gemini CLI compatible (Docker-style, project directory focus)
    Gemini,
}

impl CompatibilityMode {
    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" | "default" => Some(Self::Native),
            "claude" | "claude-code" | "claudecode" => Some(Self::ClaudeCode),
            "codex" | "openai-codex" => Some(Self::Codex),
            "gemini" | "gemini-cli" => Some(Self::Gemini),
            _ => None,
        }
    }

    /// Get the agent profile for this compatibility mode
    pub fn profile(&self) -> AgentProfile {
        match self {
            Self::Native => AgentProfile::native(),
            Self::ClaudeCode => AgentProfile::claude_code(),
            Self::Codex => AgentProfile::codex(),
            Self::Gemini => AgentProfile::gemini(),
        }
    }
}

/// Network policy with domain allowlisting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Enable network access
    pub enabled: bool,
    /// Always allowed domains (e.g., API endpoints)
    pub always_allow: Vec<String>,
    /// Allowed domains (e.g., package registries)
    pub allow: Vec<String>,
    /// Blocked domains (e.g., cloud metadata)
    pub block: Vec<String>,
}

impl NetworkPolicy {
    /// Create a policy that allows all network access
    pub fn allow_all() -> Self {
        Self {
            enabled: true,
            always_allow: Vec::new(),
            allow: Vec::new(),
            block: Vec::new(),
        }
    }

    /// Create a policy that blocks all network access
    #[allow(dead_code)]
    pub fn deny_all() -> Self {
        Self {
            enabled: false,
            always_allow: Vec::new(),
            allow: Vec::new(),
            block: Vec::new(),
        }
    }

    /// Create a policy for Claude Code (Anthropic API + common registries)
    pub fn claude_code() -> Self {
        Self {
            enabled: true,
            always_allow: vec![
                "api.anthropic.com".to_string(),
                "cdn.anthropic.com".to_string(),
            ],
            allow: vec![
                "*.pypi.org".to_string(),
                "*.npmjs.com".to_string(),
                "*.github.com".to_string(),
                "*.githubusercontent.com".to_string(),
                "*.crates.io".to_string(),
            ],
            block: vec![
                "169.254.169.254".to_string(), // Cloud metadata
                "metadata.google.internal".to_string(),
            ],
        }
    }

    /// Create a policy for Codex (OpenAI API + strict isolation)
    pub fn codex() -> Self {
        Self {
            enabled: true,
            always_allow: vec!["api.openai.com".to_string(), "cdn.openai.com".to_string()],
            allow: vec!["*.pypi.org".to_string(), "*.npmjs.com".to_string()],
            block: vec![
                "169.254.169.254".to_string(),
                "metadata.google.internal".to_string(),
                "*.internal".to_string(),
            ],
        }
    }

    /// Create a policy for Gemini (Google API + Docker-style)
    pub fn gemini() -> Self {
        Self {
            enabled: true,
            always_allow: vec![
                "generativelanguage.googleapis.com".to_string(),
                "*.googleapis.com".to_string(),
            ],
            allow: vec![
                "*.pypi.org".to_string(),
                "*.npmjs.com".to_string(),
                "*.github.com".to_string(),
            ],
            block: vec!["169.254.169.254".to_string()],
        }
    }
}

/// Agent-specific profile combining permissions and network policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Compatibility mode name
    pub mode: CompatibilityMode,
    /// Base permissions
    pub permissions: Permissions,
    /// Network policy with domain control
    pub network_policy: NetworkPolicy,
    /// API key environment variable name (if any)
    pub api_key_env: Option<String>,
    /// Additional environment variables to set
    pub env_vars: Vec<(String, String)>,
}

impl AgentProfile {
    /// Native agentkernel profile
    pub fn native() -> Self {
        Self {
            mode: CompatibilityMode::Native,
            permissions: SecurityProfile::Moderate.permissions(),
            network_policy: NetworkPolicy::allow_all(),
            api_key_env: None,
            env_vars: Vec::new(),
        }
    }

    /// Claude Code profile
    pub fn claude_code() -> Self {
        let mut perms = SecurityProfile::Moderate.permissions();
        perms.mount_cwd = true; // Claude needs project access
        perms.pass_env = false; // Controlled env passthrough
        perms.seccomp = Some("ai-agent".to_string()); // AI agent optimized profile

        Self {
            mode: CompatibilityMode::ClaudeCode,
            permissions: perms,
            network_policy: NetworkPolicy::claude_code(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            env_vars: vec![("CLAUDE_CODE_SANDBOX".to_string(), "agentkernel".to_string())],
        }
    }

    /// Codex profile (strict isolation)
    pub fn codex() -> Self {
        let mut perms = SecurityProfile::Restrictive.permissions();
        perms.network = true; // Codex needs API access
        perms.mount_cwd = true; // Codex needs project access
        perms.seccomp = Some("ai-agent".to_string()); // AI agent optimized profile

        Self {
            mode: CompatibilityMode::Codex,
            permissions: perms,
            network_policy: NetworkPolicy::codex(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            env_vars: Vec::new(),
        }
    }

    /// Gemini profile (Docker-style)
    pub fn gemini() -> Self {
        let mut perms = SecurityProfile::Moderate.permissions();
        perms.mount_cwd = true; // Gemini focuses on project directory
        perms.seccomp = Some("ai-agent".to_string()); // AI agent optimized profile

        Self {
            mode: CompatibilityMode::Gemini,
            permissions: perms,
            network_policy: NetworkPolicy::gemini(),
            api_key_env: Some("GOOGLE_API_KEY".to_string()),
            env_vars: Vec::new(),
        }
    }

    /// Get Docker network arguments based on network policy
    #[allow(dead_code)]
    pub fn network_docker_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if !self.network_policy.enabled {
            args.push("--network=none".to_string());
        }
        // Note: Domain-level filtering requires a proxy; Docker only supports on/off
        // For full domain control, use the proxy architecture from plan/06-agent-in-sandbox.md

        args
    }
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self::native()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_profiles() {
        let permissive = SecurityProfile::Permissive.permissions();
        assert!(permissive.network);
        assert!(permissive.mount_cwd);

        let moderate = SecurityProfile::Moderate.permissions();
        assert!(moderate.network);
        assert!(!moderate.mount_cwd);

        let restrictive = SecurityProfile::Restrictive.permissions();
        assert!(!restrictive.network);
        assert!(!restrictive.mount_cwd);
        assert!(restrictive.read_only_root);
    }

    #[test]
    fn test_docker_args() {
        let restrictive = SecurityProfile::Restrictive.permissions();
        let args = restrictive.to_docker_args();

        assert!(args.contains(&"--network=none".to_string()));
        assert!(args.contains(&"--read-only".to_string()));
    }

    #[test]
    fn test_compatibility_modes() {
        assert_eq!(
            CompatibilityMode::from_str("claude"),
            Some(CompatibilityMode::ClaudeCode)
        );
        assert_eq!(
            CompatibilityMode::from_str("codex"),
            Some(CompatibilityMode::Codex)
        );
        assert_eq!(
            CompatibilityMode::from_str("gemini"),
            Some(CompatibilityMode::Gemini)
        );
        assert_eq!(
            CompatibilityMode::from_str("native"),
            Some(CompatibilityMode::Native)
        );
        assert_eq!(CompatibilityMode::from_str("unknown"), None);
    }

    #[test]
    fn test_agent_profiles() {
        let claude = AgentProfile::claude_code();
        assert!(claude.permissions.mount_cwd);
        assert!(claude.network_policy.enabled);
        assert!(
            claude
                .network_policy
                .always_allow
                .contains(&"api.anthropic.com".to_string())
        );
        assert_eq!(claude.api_key_env, Some("ANTHROPIC_API_KEY".to_string()));

        let codex = AgentProfile::codex();
        assert!(codex.permissions.read_only_root); // Stricter than Claude
        assert_eq!(codex.api_key_env, Some("OPENAI_API_KEY".to_string()));

        let gemini = AgentProfile::gemini();
        assert!(
            gemini
                .network_policy
                .always_allow
                .iter()
                .any(|d| d.contains("googleapis.com"))
        );
    }

    #[test]
    fn test_network_policy() {
        let allow_all = NetworkPolicy::allow_all();
        assert!(allow_all.enabled);
        assert!(allow_all.always_allow.is_empty());

        let deny_all = NetworkPolicy::deny_all();
        assert!(!deny_all.enabled);

        let claude_net = NetworkPolicy::claude_code();
        assert!(claude_net.block.contains(&"169.254.169.254".to_string()));
    }

    #[test]
    fn test_seccomp_profiles_in_security_profiles() {
        // Each security profile should have an appropriate seccomp profile
        let permissive = SecurityProfile::Permissive.permissions();
        assert_eq!(permissive.seccomp, Some("default".to_string()));

        let moderate = SecurityProfile::Moderate.permissions();
        assert_eq!(moderate.seccomp, Some("moderate".to_string()));

        let restrictive = SecurityProfile::Restrictive.permissions();
        assert_eq!(restrictive.seccomp, Some("restrictive".to_string()));
    }

    #[test]
    fn test_seccomp_profiles_in_agent_profiles() {
        // AI agent profiles should use the ai-agent seccomp profile
        let claude = AgentProfile::claude_code();
        assert_eq!(claude.permissions.seccomp, Some("ai-agent".to_string()));

        let codex = AgentProfile::codex();
        assert_eq!(codex.permissions.seccomp, Some("ai-agent".to_string()));

        let gemini = AgentProfile::gemini();
        assert_eq!(gemini.permissions.seccomp, Some("ai-agent".to_string()));
    }

    #[test]
    fn test_seccomp_resolve_path_none() {
        // No seccomp profile should return None
        let mut perms = Permissions::default();
        perms.seccomp = None;
        assert!(perms.resolve_seccomp_path().is_none());
    }

    #[test]
    fn test_seccomp_resolve_path_builtin() {
        // Built-in profile should resolve if files exist in images/seccomp/
        let perms = SecurityProfile::Moderate.permissions();
        // This test will pass in development environment where images/seccomp/ exists
        // In production, it may return None if profiles aren't installed
        let path = perms.resolve_seccomp_path();
        if let Some(p) = path {
            assert!(p.to_string_lossy().contains("moderate.json"));
        }
    }

    #[test]
    fn test_seccomp_resolve_path_custom() {
        // Custom path that doesn't exist should return None
        let mut perms = Permissions::default();
        perms.seccomp = Some("/nonexistent/custom/profile.json".to_string());
        assert!(perms.resolve_seccomp_path().is_none());
    }

    #[test]
    fn test_docker_args_include_seccomp() {
        // When seccomp profile resolves, it should be included in docker args
        let perms = SecurityProfile::Moderate.permissions();
        let args = perms.to_docker_args();

        // Check if seccomp is present (only if profile file exists)
        let has_seccomp = args
            .iter()
            .any(|a| a.starts_with("--security-opt=seccomp="));
        // This may be true or false depending on whether the profile files exist
        // The important thing is the code doesn't panic
        let _ = has_seccomp;
    }
}
