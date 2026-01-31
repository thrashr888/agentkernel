//! Agentkernel library
//!
//! Run AI coding agents in secure, isolated microVMs.

pub mod asciicast;
pub mod audit;
pub mod backend;
pub mod build;
pub mod config;
pub mod docker_backend;
pub mod firecracker_client;
pub mod hyperlight_backend;
pub mod languages;
pub mod permissions;
pub mod rootfs;
pub mod sandbox_pool;
pub mod vsock;

// Enterprise modules (behind feature flag)
#[cfg(feature = "enterprise")]
pub mod identity;
#[cfg(feature = "enterprise")]
pub mod policy;
