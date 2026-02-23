//! Execution receipts for verifiable command runs.
//!
//! A receipt captures invocation metadata, outcome, and a tamper-evident hash.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

const RECEIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    pub version: u32,
    pub receipt_id: String,
    pub recorded_at: String,
    pub invocation: Invocation,
    pub outcome: ExecutionOutcome,
    pub payload_hash_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", content = "input", rename_all = "snake_case")]
pub enum Invocation {
    Run(RunInvocation),
    Exec(ExecInvocation),
}

impl Invocation {
    pub fn mode_name(&self) -> &'static str {
        match self {
            Invocation::Run(_) => "run",
            Invocation::Exec(_) => "exec",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInvocation {
    pub command: Vec<String>,
    pub image: Option<String>,
    pub backend: Option<String>,
    pub profile: String,
    pub no_network: bool,
    pub fast: bool,
    pub keep: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecInvocation {
    pub name: String,
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub workdir: Option<String>,
    pub sudo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub exit_code: i32,
    pub success: bool,
    pub output_sha256: String,
    pub output_bytes: usize,
    pub error: Option<String>,
}

impl ExecutionOutcome {
    pub fn from_combined_output(exit_code: i32, output: &str, error: Option<String>) -> Self {
        Self {
            exit_code,
            success: exit_code == 0,
            output_sha256: hash_output(output),
            output_bytes: output.len(),
            error,
        }
    }
}

#[derive(Serialize)]
struct HashableReceipt<'a> {
    version: u32,
    receipt_id: &'a str,
    recorded_at: &'a str,
    invocation: &'a Invocation,
    outcome: &'a ExecutionOutcome,
}

impl ExecutionReceipt {
    pub fn new(invocation: Invocation, outcome: ExecutionOutcome) -> Result<Self> {
        let mut receipt = Self {
            version: RECEIPT_VERSION,
            receipt_id: uuid::Uuid::now_v7().to_string(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
            invocation,
            outcome,
            payload_hash_sha256: String::new(),
        };
        receipt.payload_hash_sha256 = receipt.compute_payload_hash()?;
        Ok(receipt)
    }

    fn compute_payload_hash(&self) -> Result<String> {
        let hashable = HashableReceipt {
            version: self.version,
            receipt_id: &self.receipt_id,
            recorded_at: &self.recorded_at,
            invocation: &self.invocation,
            outcome: &self.outcome,
        };
        let bytes = serde_json::to_vec(&hashable).context("Failed to serialize receipt payload")?;
        Ok(hash_bytes(&bytes))
    }
}

pub fn hash_output(output: &str) -> String {
    hash_bytes(output.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{:x}", digest)
}

pub fn write_receipt(path: &Path, receipt: &ExecutionReceipt) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create receipt directory {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(receipt)?;
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write receipt {}", path.display()))?;
    Ok(())
}

pub fn load_receipt(path: &Path) -> Result<ExecutionReceipt> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read receipt {}", path.display()))?;
    let receipt: ExecutionReceipt = serde_json::from_str(&content)
        .with_context(|| format!("Invalid receipt JSON in {}", path.display()))?;
    Ok(receipt)
}

pub fn verify_receipt(receipt: &ExecutionReceipt) -> Result<()> {
    if receipt.version != RECEIPT_VERSION {
        bail!(
            "Unsupported receipt version {} (expected {})",
            receipt.version,
            RECEIPT_VERSION
        );
    }

    let expected = receipt.compute_payload_hash()?;
    if expected != receipt.payload_hash_sha256 {
        bail!(
            "Receipt hash mismatch: expected {}, found {}",
            expected,
            receipt.payload_hash_sha256
        );
    }
    Ok(())
}

pub fn verify_receipt_file(path: &Path) -> Result<ExecutionReceipt> {
    let receipt = load_receipt(path)?;
    verify_receipt(&receipt)?;
    Ok(receipt)
}

pub fn replay_args(receipt: &ExecutionReceipt) -> Vec<String> {
    match &receipt.invocation {
        Invocation::Run(run) => {
            let mut args = vec!["run".to_string()];
            if let Some(image) = &run.image {
                args.push("--image".to_string());
                args.push(image.clone());
            }
            if let Some(backend) = &run.backend {
                args.push("-B".to_string());
                args.push(backend.clone());
            }
            if run.fast {
                args.push("--fast".to_string());
            }
            if run.keep {
                args.push("--keep".to_string());
            }
            if run.no_network {
                args.push("--no-network".to_string());
            }
            args.push("--profile".to_string());
            args.push(run.profile.clone());
            args.push("--".to_string());
            args.extend(run.command.clone());
            args
        }
        Invocation::Exec(exec) => {
            let mut args = vec!["exec".to_string(), exec.name.clone()];
            for env in &exec.env {
                args.push("--env".to_string());
                args.push(env.clone());
            }
            if let Some(workdir) = &exec.workdir {
                args.push("--workdir".to_string());
                args.push(workdir.clone());
            }
            if exec.sudo {
                args.push("--sudo".to_string());
            }
            args.push("--".to_string());
            args.extend(exec.command.clone());
            args
        }
    }
}
