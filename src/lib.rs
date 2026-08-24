//! Agentkernel library
//!
//! Run AI coding agents in secure, isolated microVMs.

pub mod asciicast;
pub mod audit;
pub mod backend;
pub mod build;
pub mod config;
pub mod container_network;
pub mod cow;
pub mod docker_backend;
pub mod durable_storage;
pub mod firecracker_client;
pub mod full_state;
mod git_worktree;
pub mod hyperlight_backend;
pub mod interactive_permissions;
pub mod languages;
pub mod llm_intercept;
pub mod llm_spend;
pub mod metrics;
pub mod model_governance;
pub mod orchestration_store;
pub mod permissions;
mod pool;
pub mod proxy;
pub mod proxy_hooks;
pub mod rootfs;
pub mod sandbox_pool;
#[allow(dead_code)]
mod secrets;
mod secure_fs;
pub mod ssh;
pub mod task_coordinator;
pub mod task_worker;
pub mod tasks;
pub mod tls;
pub mod validation;
#[allow(dead_code)]
pub mod vmm;
#[allow(dead_code)]
mod volume;
pub mod vsock;
pub mod vsock_secrets;

// Enterprise modules (behind feature flag)
#[cfg(feature = "enterprise")]
pub mod identity;
#[cfg(feature = "enterprise")]
pub mod policy;
#[cfg(feature = "enterprise")]
#[allow(dead_code)]
mod quota;
