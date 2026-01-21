//! Health checking for daemon VMs.
//!
//! Most health checking logic is integrated into the pool,
//! but this module provides utilities for explicit health checks.

#![allow(dead_code)]

use anyhow::Result;
use std::path::Path;

use crate::vsock::VsockClient;

/// Check if a VM is healthy by pinging its guest agent
pub async fn check_vm_health(vsock_path: &Path) -> Result<bool> {
    let client = VsockClient::for_firecracker(vsock_path.to_path_buf());
    Ok(client.ping().await.unwrap_or(false))
}

/// Health status for a VM
#[derive(Debug, Clone)]
pub struct VmHealth {
    /// VM ID
    pub id: String,
    /// Is the VM process alive
    pub process_alive: bool,
    /// Is the guest agent responding
    pub agent_responding: bool,
    /// Age in seconds
    pub age_secs: u64,
}

impl VmHealth {
    /// Check if VM is fully healthy
    pub fn is_healthy(&self) -> bool {
        self.process_alive && self.agent_responding
    }
}
