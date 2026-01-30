//! # agentkernel-sdk
//!
//! Rust SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run
//! AI coding agents in secure, isolated microVMs.
//!
//! ## Quick Start
//!
//! ```no_run
//! # async fn example() -> agentkernel_sdk::Result<()> {
//! use agentkernel_sdk::AgentKernel;
//!
//! let client = AgentKernel::builder().build()?;
//! let output = client.run(&["echo", "hello"], None).await?;
//! println!("{}", output.output);
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod types;

pub use client::{AgentKernel, AgentKernelBuilder, SandboxHandle};
pub use error::{Error, Result};
pub use types::{
    BatchCommand, BatchResult, BatchRunResponse, FileReadResponse, RunOptions, RunOutput,
    SandboxInfo, SecurityProfile, StreamEvent,
};
