//! Configuration parsing for agentkernel.toml files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub name: String,
    /// Runtime image: base, python, node, go, rust, or path to custom rootfs
    #[serde(default = "default_runtime")]
    pub runtime: String,
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
            },
            agent: AgentConfig {
                preferred: agent.to_string(),
            },
            resources: ResourcesConfig::default(),
            network: NetworkConfig::default(),
        }
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
}
