//! JSON protocol for daemon communication.

use serde::{Deserialize, Serialize};

/// Request from CLI to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// Acquire a VM from the pool
    Acquire {
        /// Runtime type (base, python, node, etc.)
        runtime: String,
    },
    /// Release a VM back to the pool
    Release {
        /// VM ID to release
        id: String,
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
        /// CID for vsock communication
        cid: u32,
        /// Path to vsock UDS
        vsock_path: String,
    },
    /// VM released successfully
    Released,
    /// Daemon status
    Status {
        /// Number of warm VMs in pool
        warm: usize,
        /// Number of VMs currently in use
        in_use: usize,
        /// Pool configuration
        min_warm: usize,
        max_warm: usize,
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
