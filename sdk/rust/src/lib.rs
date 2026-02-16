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

pub mod browser;
mod client;
mod error;
mod types;

pub use browser::BrowserSession;
pub use client::{AgentKernel, AgentKernelBuilder, SandboxHandle};
pub use error::{Error, Result};
pub use types::{
    AriaSnapshot, BatchCommand, BatchFileWriteResponse, BatchResult, BatchRunResponse,
    BrowserEvent, CreateSandboxOptions, DetachedCommand, DetachedLogsResponse, DetachedStatus,
    DurableObject, DurableObjectCreateRequest, ExecOptions, ExtendTtlResponse, FileReadResponse,
    Orchestration, OrchestrationCreateRequest, PageLink, PageResult, RunOptions, RunOutput,
    Schedule, ScheduleCreateRequest, SandboxInfo, SecurityProfile, SnapshotMeta, StreamEvent,
    TakeSnapshotOptions,
};
