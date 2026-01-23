//! JSON protocol for daemon communication.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Backend type for daemon requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DaemonBackend {
    /// Firecracker microVM (default)
    #[default]
    Firecracker,
    /// Hyperlight WebAssembly
    Hyperlight,
    /// Docker container
    Docker,
    /// Apple Containers
    Apple,
}

/// Agent compatibility mode for daemon requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DaemonCompatibilityMode {
    /// Default agentkernel behavior
    #[default]
    Native,
    /// Claude Code compatible
    Claude,
    /// OpenAI Codex compatible
    Codex,
    /// Gemini CLI compatible
    Gemini,
}

/// Request from CLI to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// Acquire a VM from the pool
    Acquire {
        /// Runtime type (base, python, node, etc.)
        runtime: String,
        /// Backend to use (optional, defaults to Firecracker)
        #[serde(default)]
        backend: DaemonBackend,
        /// Agent compatibility mode (optional, defaults to Native)
        #[serde(default)]
        compatibility_mode: DaemonCompatibilityMode,
    },
    /// Release a VM back to the pool
    Release {
        /// VM ID to release
        id: String,
    },
    /// Execute a command in a pooled VM (acquire + exec + release in one call)
    Exec {
        /// Runtime type (base, python, node, etc.)
        runtime: String,
        /// Command to execute
        command: Vec<String>,
        /// Backend to use (optional, defaults to Firecracker)
        #[serde(default)]
        backend: DaemonBackend,
        /// Agent compatibility mode (optional, defaults to Native)
        #[serde(default)]
        compatibility_mode: DaemonCompatibilityMode,
    },
    /// Pre-warm the pool for a specific agent type
    Prewarm {
        /// Agent compatibility mode to pre-warm for
        compatibility_mode: DaemonCompatibilityMode,
    },
    /// Get daemon status
    Status,
    /// Shutdown the daemon
    Shutdown,
}

/// Response from daemon to CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonResponse {
    /// VM acquired successfully
    Acquired {
        /// VM ID
        id: String,
        /// CID for vsock communication (Firecracker only)
        #[serde(skip_serializing_if = "Option::is_none")]
        cid: Option<u32>,
        /// Path to vsock UDS (Firecracker only)
        #[serde(skip_serializing_if = "Option::is_none")]
        vsock_path: Option<String>,
        /// Backend type used
        backend: DaemonBackend,
    },
    /// VM released successfully
    Released,
    /// Command executed successfully
    Executed {
        /// Exit code from command
        exit_code: i32,
        /// Standard output
        stdout: String,
        /// Standard error
        stderr: String,
    },
    /// Daemon status
    Status {
        /// Number of warm VMs in pool
        warm: usize,
        /// Number of VMs currently in use
        in_use: usize,
        /// Pool configuration
        min_warm: usize,
        max_warm: usize,
        /// Supported backends
        backends: Vec<String>,
        /// Per-agent pool stats (warm count per compatibility mode)
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        agent_stats: HashMap<String, usize>,
    },
    /// Pool pre-warmed for agent
    Prewarmed {
        /// Agent mode that was pre-warmed
        compatibility_mode: DaemonCompatibilityMode,
        /// Number of VMs created
        count: usize,
    },
    /// Shutdown acknowledged
    ShuttingDown,
    /// Error response
    Error {
        /// Error message
        message: String,
    },
}

impl DaemonResponse {
    /// Create an error response
    pub fn error(message: impl Into<String>) -> Self {
        DaemonResponse::Error {
            message: message.into(),
        }
    }
}
