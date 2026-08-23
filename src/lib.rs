//! Agentkernel library
//!
//! Run AI coding agents in secure, isolated microVMs.

pub mod asciicast;
pub mod audit;
pub mod backend;
pub mod build;
pub mod config;
pub mod docker_backend;
pub mod durable_storage;
pub mod firecracker_client;
pub mod hyperlight_backend;
pub mod interactive_permissions;
pub mod languages;
pub mod llm_intercept;
pub mod metrics;
pub mod orchestration_store;
pub mod permissions;
pub mod proxy;
pub mod proxy_hooks;
pub mod rootfs;
pub mod sandbox_pool;
pub mod ssh;
pub mod task_worker;
pub mod tasks;
pub mod tls;
pub mod validation;
pub mod vsock;
pub mod vsock_secrets;

// Enterprise modules (behind feature flag)
#[cfg(feature = "enterprise")]
pub mod identity;
#[cfg(feature = "enterprise")]
pub mod policy;
