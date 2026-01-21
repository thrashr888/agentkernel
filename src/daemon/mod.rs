//! Daemon mode for Firecracker VM pool management.
//!
//! The daemon maintains a pool of pre-warmed Firecracker VMs for fast execution.
//! The CLI connects to the daemon via Unix socket to acquire VMs from the pool.

mod client;
mod health;
mod pool;
mod protocol;
mod server;

pub use client::DaemonClient;
pub use pool::PoolConfig;
pub use server::DaemonServer;
