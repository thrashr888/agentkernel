//! Multi-agent support for agentkernel.
//!
//! Provides adapters for different AI coding agents: Claude Code, Gemini CLI, Codex, OpenCode.

#![allow(dead_code)]

use anyhow::Result;
use std::collections::HashMap;

/// Agent type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentType {
    #[default]
    Claude,
    Gemini,
    Codex,
    OpenCode,
}

impl AgentType {
    /// Parse agent type from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "gemini" | "gemini-cli" => Some(Self::Gemini),
            "codex" | "openai-codex" => Some(Self::Codex),
            "opencode" | "open-code" => Some(Self::OpenCode),
            _ => None,
        }
    }

    /// Get the display name for this agent
    pub fn name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Gemini => "Gemini CLI",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }

    /// Get the command to launch this agent
    pub fn command(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

/// Configuration for an agent
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AgentConfig {
    pub agent_type: AgentType,
    pub env_vars: HashMap<String, String>,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
}

#[allow(dead_code)]
impl AgentConfig {
    /// Create config for a specific agent type
    pub fn for_agent(agent_type: AgentType) -> Self {
        Self {
            agent_type,
            ..Default::default()
        }
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Add command-line arguments
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Set the working directory
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}

/// Agent adapter trait - defines how to interact with each agent
pub trait Agent {
    /// Get the agent type
    fn agent_type(&self) -> AgentType;

    /// Get the command to launch this agent
    fn launch_command(&self) -> Vec<String>;

    /// Get environment variables to set
    fn env_vars(&self) -> &HashMap<String, String>;

    /// Get the required API key environment variable name (if any)
    fn api_key_env_var(&self) -> Option<&'static str>;

    /// Check if the agent is available (installed and configured)
    fn is_available(&self) -> bool;

    /// Get install instructions
    fn install_instructions(&self) -> &'static str;
}

/// Claude Code adapter
pub struct ClaudeAgent {
    config: AgentConfig,
}

impl ClaudeAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }
}

impl Agent for ClaudeAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Claude
    }

    fn launch_command(&self) -> Vec<String> {
        let mut cmd = vec!["claude".to_string()];
        cmd.extend(self.config.args.clone());
        cmd
    }

    fn env_vars(&self) -> &HashMap<String, String> {
        &self.config.env_vars
    }

    fn api_key_env_var(&self) -> Option<&'static str> {
        Some("ANTHROPIC_API_KEY")
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("claude")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install_instructions(&self) -> &'static str {
        "Install Claude Code: npm install -g @anthropic-ai/claude-code"
    }
}

/// Gemini CLI adapter
pub struct GeminiAgent {
    config: AgentConfig,
}

impl GeminiAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }
}

impl Agent for GeminiAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Gemini
    }

    fn launch_command(&self) -> Vec<String> {
        let mut cmd = vec!["gemini".to_string()];
        cmd.extend(self.config.args.clone());
        cmd
    }

    fn env_vars(&self) -> &HashMap<String, String> {
        &self.config.env_vars
    }

    fn api_key_env_var(&self) -> Option<&'static str> {
        Some("GOOGLE_API_KEY")
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("gemini")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install_instructions(&self) -> &'static str {
        "Install Gemini CLI: pip install google-generativeai"
    }
}

/// Codex adapter
pub struct CodexAgent {
    config: AgentConfig,
}

impl CodexAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }
}

impl Agent for CodexAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Codex
    }

    fn launch_command(&self) -> Vec<String> {
        let mut cmd = vec!["codex".to_string()];
        cmd.extend(self.config.args.clone());
        cmd
    }

    fn env_vars(&self) -> &HashMap<String, String> {
        &self.config.env_vars
    }

    fn api_key_env_var(&self) -> Option<&'static str> {
        Some("OPENAI_API_KEY")
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("codex")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install_instructions(&self) -> &'static str {
        "Install Codex CLI: npm install -g @openai/codex"
    }
}

/// OpenCode adapter
pub struct OpenCodeAgent {
    config: AgentConfig,
}

impl OpenCodeAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }
}

impl Agent for OpenCodeAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::OpenCode
    }

    fn launch_command(&self) -> Vec<String> {
        let mut cmd = vec!["opencode".to_string()];
        cmd.extend(self.config.args.clone());
        cmd
    }

    fn env_vars(&self) -> &HashMap<String, String> {
        &self.config.env_vars
    }

    fn api_key_env_var(&self) -> Option<&'static str> {
        // OpenCode supports multiple providers
        None
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("opencode")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install_instructions(&self) -> &'static str {
        "Install OpenCode: cargo install opencode"
    }
}

/// Create an agent adapter from type
pub fn create_agent(agent_type: AgentType, config: Option<AgentConfig>) -> Box<dyn Agent> {
    let config = config.unwrap_or_else(|| AgentConfig::for_agent(agent_type));

    match agent_type {
        AgentType::Claude => Box::new(ClaudeAgent::new(config)),
        AgentType::Gemini => Box::new(GeminiAgent::new(config)),
        AgentType::Codex => Box::new(CodexAgent::new(config)),
        AgentType::OpenCode => Box::new(OpenCodeAgent::new(config)),
    }
}

/// Create an agent from a string type name
pub fn create_agent_from_str(agent_name: &str) -> Result<Box<dyn Agent>> {
    let agent_type = AgentType::from_str(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown agent type: {}", agent_name))?;
    Ok(create_agent(agent_type, None))
}

/// Check agent availability
pub fn check_agent_availability(agent_type: AgentType) -> AgentStatus {
    let agent = create_agent(agent_type, None);

    let installed = agent.is_available();
    let api_key_set = agent
        .api_key_env_var()
        .map(|var| std::env::var(var).is_ok())
        .unwrap_or(true);

    AgentStatus {
        agent_type,
        installed,
        api_key_set,
        install_instructions: agent.install_instructions().to_string(),
    }
}

/// Status of an agent's availability
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub agent_type: AgentType,
    pub installed: bool,
    pub api_key_set: bool,
    pub install_instructions: String,
}

impl AgentStatus {
    /// Check if agent is fully ready
    pub fn is_ready(&self) -> bool {
        self.installed && self.api_key_set
    }

    /// Print status
    pub fn print(&self) {
        let status = if self.is_ready() {
            "ready"
        } else if self.installed {
            "api key missing"
        } else {
            "not installed"
        };

        println!("{:<15} {}", self.agent_type.name(), status);

        if !self.installed {
            println!("  {}", self.install_instructions);
        }
    }
}

/// List all available agents and their status
pub fn list_agents() -> Vec<AgentStatus> {
    vec![
        check_agent_availability(AgentType::Claude),
        check_agent_availability(AgentType::Gemini),
        check_agent_availability(AgentType::Codex),
        check_agent_availability(AgentType::OpenCode),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_type_from_str() {
        assert_eq!(AgentType::from_str("claude"), Some(AgentType::Claude));
        assert_eq!(AgentType::from_str("Claude"), Some(AgentType::Claude));
        assert_eq!(AgentType::from_str("gemini"), Some(AgentType::Gemini));
        assert_eq!(AgentType::from_str("codex"), Some(AgentType::Codex));
        assert_eq!(AgentType::from_str("opencode"), Some(AgentType::OpenCode));
        assert_eq!(AgentType::from_str("unknown"), None);
    }

    #[test]
    fn test_agent_config() {
        let config = AgentConfig::for_agent(AgentType::Claude)
            .with_env("CUSTOM_VAR", "value")
            .with_args(vec!["--flag".to_string()]);

        assert_eq!(config.agent_type, AgentType::Claude);
        assert_eq!(
            config.env_vars.get("CUSTOM_VAR"),
            Some(&"value".to_string())
        );
        assert_eq!(config.args, vec!["--flag".to_string()]);
    }

    #[test]
    fn test_create_agent() {
        let agent = create_agent(AgentType::Claude, None);
        assert_eq!(agent.agent_type(), AgentType::Claude);
        assert_eq!(agent.api_key_env_var(), Some("ANTHROPIC_API_KEY"));
    }
}
