//! Configuration parsing for agentkernel.toml files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::backend::FileInjection;
use crate::permissions::SecurityProfile;

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
    /// Files to inject into the sandbox at startup
    #[serde(default, rename = "files")]
    pub files: Vec<FileEntry>,
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
    /// Check if a domain is allowed
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            preferred: default_agent(),
            compatibility_mode: None,
        }
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
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        Self::from_str(&content)
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
            },
            agent: AgentConfig {
                preferred: agent.to_string(),
                compatibility_mode: None,
            },
            resources: ResourcesConfig::default(),
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
            build: BuildConfig::default(),
            files: Vec::new(),
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
            _ => "alpine:3.20".to_string(),
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
}
