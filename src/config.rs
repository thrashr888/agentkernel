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
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            preferred: default_agent(),
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
            },
            resources: ResourcesConfig::default(),
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
            files: Vec::new(),
        }
    }

    /// Get the effective permissions based on config
    pub fn get_permissions(&self) -> crate::permissions::Permissions {
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
}
