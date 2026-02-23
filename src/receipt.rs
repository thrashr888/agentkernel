//! Execution receipts for verifiable command runs.
//!
//! A receipt captures invocation metadata, outcome, and a tamper-evident hash.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const RECEIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    pub version: u32,
    pub receipt_id: String,
    pub recorded_at: String,
    pub invocation: Invocation,
    pub outcome: ExecutionOutcome,
    pub payload_hash_sha256: String,
    pub signature: Option<ReceiptSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSignature {
    pub algorithm: String,
    pub key_id: String,
    pub public_key: String,
    pub signature: String,
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
            signature: None,
        };
        let payload = receipt.payload_bytes()?;
        receipt.payload_hash_sha256 = hash_bytes(&payload);
        receipt.signature = Some(sign_payload(&payload)?);
        Ok(receipt)
    }

    fn payload_bytes(&self) -> Result<Vec<u8>> {
        let hashable = HashableReceipt {
            version: self.version,
            receipt_id: &self.receipt_id,
            recorded_at: &self.recorded_at,
            invocation: &self.invocation,
            outcome: &self.outcome,
        };
        serde_json::to_vec(&hashable).context("Failed to serialize receipt payload")
    }
}

pub fn hash_output(output: &str) -> String {
    hash_bytes(output.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{:x}", digest)
}

fn receipts_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".agentkernel").join("receipts")
    } else {
        PathBuf::from("/tmp/agentkernel/receipts")
    }
}

fn signing_key_path() -> PathBuf {
    receipts_dir().join("ed25519_signing_key.pkcs8")
}

fn load_or_create_signing_key() -> Result<Ed25519KeyPair> {
    let path = signing_key_path();

    if path.exists() {
        let key_bytes = std::fs::read(&path)
            .with_context(|| format!("Failed to read signing key {}", path.display()))?;
        return Ed25519KeyPair::from_pkcs8(&key_bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Ed25519 signing key at {}", path.display()));
    }

    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| anyhow::anyhow!("Failed to generate Ed25519 signing key"))?;
    crate::secure_fs::write_private_file_atomic(&path, pkcs8.as_ref())
        .with_context(|| format!("Failed to write signing key {}", path.display()))?;

    let key_bytes = std::fs::read(&path)
        .with_context(|| format!("Failed to read signing key {}", path.display()))?;
    Ed25519KeyPair::from_pkcs8(&key_bytes).map_err(|_| {
        anyhow::anyhow!(
            "Invalid generated Ed25519 signing key at {}",
            path.display()
        )
    })
}

fn short_key_id(public_key: &[u8]) -> String {
    hash_bytes(public_key).chars().take(16).collect()
}

fn sign_payload(payload: &[u8]) -> Result<ReceiptSignature> {
    let keypair = load_or_create_signing_key()?;
    let public_key = keypair.public_key().as_ref().to_vec();
    let signature = keypair.sign(payload);
    Ok(ReceiptSignature {
        algorithm: "ed25519".to_string(),
        key_id: short_key_id(&public_key),
        public_key: BASE64.encode(public_key),
        signature: BASE64.encode(signature.as_ref()),
    })
}

fn verify_signature(signature: &ReceiptSignature, payload: &[u8]) -> Result<()> {
    if !signature.algorithm.eq_ignore_ascii_case("ed25519") {
        bail!(
            "Unsupported signature algorithm '{}'; expected ed25519",
            signature.algorithm
        );
    }

    let public_key = BASE64
        .decode(&signature.public_key)
        .context("Invalid base64 public key in receipt signature")?;
    let sig = BASE64
        .decode(&signature.signature)
        .context("Invalid base64 signature bytes in receipt signature")?;

    let expected_key_id = short_key_id(&public_key);
    if expected_key_id != signature.key_id {
        bail!(
            "Signature key ID mismatch: expected {}, found {}",
            expected_key_id,
            signature.key_id
        );
    }

    UnparsedPublicKey::new(&ED25519, &public_key)
        .verify(payload, &sig)
        .map_err(|_| anyhow::anyhow!("Invalid receipt signature"))?;

    Ok(())
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

pub fn verify_receipt(receipt: &ExecutionReceipt, allow_unsigned: bool) -> Result<()> {
    if receipt.version != RECEIPT_VERSION {
        bail!(
            "Unsupported receipt version {} (expected {})",
            receipt.version,
            RECEIPT_VERSION
        );
    }

    let payload = receipt.payload_bytes()?;
    let expected = hash_bytes(&payload);
    if expected != receipt.payload_hash_sha256 {
        bail!(
            "Receipt hash mismatch: expected {}, found {}",
            expected,
            receipt.payload_hash_sha256
        );
    }

    match &receipt.signature {
        Some(signature) => verify_signature(signature, &payload)?,
        None => {
            if !allow_unsigned {
                bail!("Receipt is unsigned. Pass --allow-unsigned to verify legacy receipts.");
            }
        }
    }

    Ok(())
}

pub fn verify_receipt_file(path: &Path, allow_unsigned: bool) -> Result<ExecutionReceipt> {
    let receipt = load_receipt(path)?;
    verify_receipt(&receipt, allow_unsigned)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_invocation() -> Invocation {
        Invocation::Run(RunInvocation {
            command: vec![
                "python3".to_string(),
                "-c".to_string(),
                "print('ok')".to_string(),
            ],
            image: Some("python:3.12-alpine".to_string()),
            backend: Some("docker".to_string()),
            profile: "moderate".to_string(),
            no_network: false,
            fast: false,
            keep: false,
        })
    }

    fn unsigned_receipt() -> ExecutionReceipt {
        let mut rec = ExecutionReceipt {
            version: RECEIPT_VERSION,
            receipt_id: "test-receipt-1".to_string(),
            recorded_at: "2026-02-23T00:00:00Z".to_string(),
            invocation: sample_invocation(),
            outcome: ExecutionOutcome::from_combined_output(0, "ok\n", None),
            payload_hash_sha256: String::new(),
            signature: None,
        };
        let payload = rec.payload_bytes().expect("serialize payload");
        rec.payload_hash_sha256 = hash_bytes(&payload);
        rec
    }

    #[test]
    fn verify_unsigned_receipt_requires_flag() {
        let rec = unsigned_receipt();
        assert!(verify_receipt(&rec, false).is_err());
        assert!(verify_receipt(&rec, true).is_ok());
    }

    #[test]
    fn verify_receipt_detects_tampering() {
        let mut rec = unsigned_receipt();
        if let Invocation::Run(run) = &mut rec.invocation {
            run.command.push("--tampered".to_string());
        }
        let err = verify_receipt(&rec, true).expect_err("tampered receipt should fail");
        assert!(err.to_string().contains("Receipt hash mismatch"));
    }

    #[test]
    fn write_and_load_receipt_roundtrip() {
        let rec = unsigned_receipt();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("receipt.json");

        write_receipt(&path, &rec).expect("write receipt");
        let loaded = load_receipt(&path).expect("load receipt");

        assert_eq!(loaded.version, rec.version);
        assert_eq!(loaded.receipt_id, rec.receipt_id);
        assert_eq!(loaded.payload_hash_sha256, rec.payload_hash_sha256);
        assert!(verify_receipt(&loaded, true).is_ok());
    }

    #[test]
    fn replay_args_for_exec_include_options() {
        let exec_receipt = ExecutionReceipt {
            version: RECEIPT_VERSION,
            receipt_id: "exec-test".to_string(),
            recorded_at: "2026-02-23T00:00:00Z".to_string(),
            invocation: Invocation::Exec(ExecInvocation {
                name: "dev".to_string(),
                command: vec!["pytest".to_string(), "-q".to_string()],
                env: vec!["FOO=bar".to_string(), "DEBUG=1".to_string()],
                workdir: Some("/workspace".to_string()),
                sudo: true,
            }),
            outcome: ExecutionOutcome::from_combined_output(0, "", None),
            payload_hash_sha256: String::new(),
            signature: None,
        };

        let args = replay_args(&exec_receipt);
        assert_eq!(
            args,
            vec![
                "exec",
                "dev",
                "--env",
                "FOO=bar",
                "--env",
                "DEBUG=1",
                "--workdir",
                "/workspace",
                "--sudo",
                "--",
                "pytest",
                "-q",
            ]
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
        );
    }
}
