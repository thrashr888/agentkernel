//! Configuration parsing for agentkernel.toml files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure matching agentkernel.toml schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub environment: HashMap<String, String>,
    #[serde(default)]
    pub dependencies: DependenciesConfig,
    #[serde(default)]
    pub scripts: ScriptsConfig,
    #[serde(default)]
    pub mounts: HashMap<String, String>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub resources: ResourcesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub name: String,
    #[serde(default = "default_base_image")]
    pub base_image: String,
}

fn default_base_image() -> String {
    "ubuntu:24.04".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependenciesConfig {
    #[serde(default)]
    pub system: Vec<String>,
    #[serde(default)]
    pub python: Vec<String>,
    #[serde(default)]
    pub node: Vec<String>,
    #[serde(default)]
    pub rust: Vec<String>,
    #[serde(default)]
    pub go: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptsConfig {
    pub setup: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
    pub build: Option<String>,
    pub run: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesConfig {
    /// Memory limit in MB (default: 2048)
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u64,
    /// CPU limit as fraction of cores (default: 2.0)
    #[serde(default = "default_cpu_limit")]
    pub cpu_limit: f64,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            memory_mb: default_memory_mb(),
            cpu_limit: default_cpu_limit(),
        }
    }
}

fn default_memory_mb() -> u64 {
    2048
}

fn default_cpu_limit() -> f64 {
    2.0
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
                base_image: default_base_image(),
            },
            agent: AgentConfig {
                preferred: agent.to_string(),
            },
            environment: HashMap::new(),
            dependencies: DependenciesConfig::default(),
            scripts: ScriptsConfig::default(),
            mounts: HashMap::new(),
            network: NetworkConfig::default(),
            resources: ResourcesConfig::default(),
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
        assert_eq!(config.sandbox.base_image, "ubuntu:24.04");
        assert_eq!(config.agent.preferred, "claude");
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            [sandbox]
            name = "python-app"
            base_image = "python:3.12-slim"

            [agent]
            preferred = "gemini"

            [dependencies]
            system = ["git", "curl"]
            python = ["flask", "pytest"]

            [scripts]
            test = "pytest"
            lint = "ruff check ."

            [mounts]
            "." = "/app"

            [network]
            ports = [5000, 8080]
        "#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.sandbox.name, "python-app");
        assert_eq!(config.sandbox.base_image, "python:3.12-slim");
        assert_eq!(config.agent.preferred, "gemini");
        assert_eq!(config.dependencies.system, vec!["git", "curl"]);
        assert_eq!(config.dependencies.python, vec!["flask", "pytest"]);
        assert_eq!(config.scripts.test, Some("pytest".to_string()));
        assert_eq!(config.mounts.get("."), Some(&"/app".to_string()));
        assert_eq!(config.network.ports, vec![5000, 8080]);
    }
}
