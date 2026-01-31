//! HashiCorp Nomad backend implementing the Sandbox trait.
//!
//! Each sandbox is a Nomad batch job. start() submits a job with `sleep infinity`,
//! exec() shells out to `nomad alloc exec` (Phase 1), stop() purges the job.
//!
//! Compile with `--features nomad` to enable.

#![cfg(feature = "nomad")]

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::json;
use std::process::Command;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};
use crate::config::OrchestratorConfig;

/// HTTP client for the Nomad API
struct NomadClient {
    addr: String,
    token: Option<String>,
    #[allow(dead_code)]
    region: Option<String>,
    http: reqwest::Client,
}

impl NomadClient {
    fn new(config: &OrchestratorConfig) -> Self {
        let addr = config
            .nomad_addr
            .clone()
            .or_else(|| std::env::var("NOMAD_ADDR").ok())
            .unwrap_or_else(|| "http://127.0.0.1:4646".to_string());

        let token = config
            .nomad_token
            .clone()
            .or_else(|| std::env::var("NOMAD_TOKEN").ok());

        Self {
            addr,
            token,
            region: None,
            http: reqwest::Client::new(),
        }
    }

    /// Make a GET request to the Nomad API
    async fn get(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.get(&url);

        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }

        let resp = req.send().await.context("Nomad API request failed")?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error ({}): {}", status, body);
        }

        resp.json().await.context("Failed to parse Nomad response")
    }

    /// Make a PUT request to the Nomad API
    async fn put(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.put(&url).json(body);

        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }

        let resp = req.send().await.context("Nomad API PUT failed")?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error ({}): {}", status, body);
        }

        resp.json().await.context("Failed to parse Nomad response")
    }

    /// Make a DELETE request to the Nomad API
    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.delete(&url);

        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }

        let resp = req.send().await.context("Nomad API DELETE failed")?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error ({}): {}", status, body);
        }

        Ok(())
    }
}

/// Nomad job-based sandbox
pub struct NomadSandbox {
    /// Sandbox name
    name: String,
    /// Nomad job ID (set after start())
    job_id: Option<String>,
    /// Nomad allocation ID (set after start(), used for exec)
    alloc_id: Option<String>,
    /// Whether the sandbox is running
    running: bool,
    /// Nomad API client
    client: NomadClient,
    /// Task driver: "docker", "exec", "raw_exec"
    driver: String,
    /// Nomad datacenter
    datacenter: Option<String>,
}

impl NomadSandbox {
    /// Create a new Nomad sandbox from orchestrator configuration
    pub fn new(name: &str, config: &OrchestratorConfig) -> Self {
        Self {
            name: name.to_string(),
            job_id: None,
            alloc_id: None,
            running: false,
            client: NomadClient::new(config),
            driver: config.nomad_driver.clone(),
            datacenter: config.nomad_datacenter.clone(),
        }
    }

    /// Generate the job ID for this sandbox
    fn job_id_for(sandbox_name: &str) -> String {
        let sanitized: String = sandbox_name
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        format!("agentkernel-{}", sanitized)
    }

    /// Build the Nomad job spec
    fn build_job_spec(&self, config: &SandboxConfig) -> serde_json::Value {
        let job_id = Self::job_id_for(&self.name);
        let datacenter = self.datacenter.clone().unwrap_or_else(|| "dc1".to_string());

        // Build driver-specific config
        let driver_config = match self.driver.as_str() {
            "docker" => {
                let mut cfg = json!({
                    "image": config.image,
                    "command": "sh",
                    "args": ["-c", "sleep infinity"],
                });
                if config.read_only {
                    cfg["readonly_rootfs"] = json!(true);
                }
                if !config.network {
                    cfg["network_mode"] = json!("none");
                }
                // Drop all capabilities for security
                cfg["cap_drop"] = json!(["ALL"]);
                cfg["privileged"] = json!(false);
                cfg
            }
            "exec" | "raw_exec" => {
                json!({
                    "command": "sh",
                    "args": ["-c", "sleep infinity"],
                })
            }
            _ => {
                json!({
                    "command": "sh",
                    "args": ["-c", "sleep infinity"],
                })
            }
        };

        // Build network stanza
        let network = if !config.network {
            json!({ "mode": "none" })
        } else {
            json!({})
        };

        json!({
            "Job": {
                "ID": job_id,
                "Name": job_id,
                "Type": "batch",
                "Datacenters": [datacenter],
                "Meta": {
                    "agentkernel-sandbox": self.name,
                    "agentkernel-managed": "true"
                },
                "TaskGroups": [{
                    "Name": "sandbox",
                    "Count": 1,
                    "Networks": [network],
                    "Tasks": [{
                        "Name": "sandbox",
                        "Driver": self.driver,
                        "Config": driver_config,
                        "Resources": {
                            "CPU": config.vcpus * 1000,
                            "MemoryMB": config.memory_mb
                        },
                        "Meta": {
                            "agentkernel-sandbox": self.name,
                            "agentkernel-managed": "true"
                        }
                    }]
                }]
            }
        })
    }

    /// Wait for an allocation to be in the running state
    async fn wait_for_running(&self, job_id: &str) -> Result<String> {
        // Poll for up to 120 seconds
        for _ in 0..240 {
            let allocs = self
                .client
                .get(&format!("/v1/job/{}/allocations", job_id))
                .await?;

            if let Some(allocs) = allocs.as_array() {
                for alloc in allocs {
                    let status = alloc
                        .get("ClientStatus")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let alloc_id = alloc.get("ID").and_then(|s| s.as_str()).unwrap_or("");

                    match status {
                        "running" => return Ok(alloc_id.to_string()),
                        "failed" | "lost" => {
                            bail!("Nomad allocation entered {} state", status);
                        }
                        _ => {} // pending, etc.
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        bail!("Timed out waiting for Nomad allocation to start")
    }

    /// Run a command via `nomad alloc exec` (Phase 1: shell out to CLI).
    ///
    /// Uses `std::process::Command` with explicit arguments to avoid shell injection.
    fn run_nomad_exec(
        alloc_id: &str,
        cmd: &[&str],
        nomad_addr: &str,
        nomad_token: &Option<String>,
    ) -> Result<ExecResult> {
        let mut command = Command::new("nomad");
        command.arg("alloc").arg("exec").arg(alloc_id);

        // Set NOMAD_ADDR env
        command.env("NOMAD_ADDR", nomad_addr);

        // Set NOMAD_TOKEN env if present
        if let Some(ref token) = nomad_token {
            command.env("NOMAD_TOKEN", token);
        }

        // Add the separator and command arguments individually
        command.arg("--");
        for arg in cmd {
            command.arg(arg);
        }

        let output = command.output().context("Failed to run nomad alloc exec")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(1);

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }
}

#[async_trait]
impl Sandbox for NomadSandbox {
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        let job_spec = self.build_job_spec(config);
        let job_id = Self::job_id_for(&self.name);

        // Submit the job
        self.client
            .put("/v1/jobs", &job_spec)
            .await
            .context("Failed to submit Nomad job")?;

        // Wait for an allocation to be running
        let alloc_id = self.wait_for_running(&job_id).await?;

        self.job_id = Some(job_id);
        self.alloc_id = Some(alloc_id);
        self.running = true;

        Ok(())
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.exec_with_env(cmd, &[]).await
    }

    async fn exec_with_env(&mut self, cmd: &[&str], env: &[String]) -> Result<ExecResult> {
        let alloc_id = self
            .alloc_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Nomad allocation not started"))?;

        // Wrap command with env if provided
        let full_cmd: Vec<String> = if env.is_empty() {
            cmd.iter().map(|s| s.to_string()).collect()
        } else {
            let mut parts = vec!["env".to_string()];
            parts.extend(env.iter().cloned());
            parts.extend(cmd.iter().map(|s| s.to_string()));
            parts
        };

        let cmd_refs: Vec<&str> = full_cmd.iter().map(|s| s.as_str()).collect();

        // Phase 1: Shell out to nomad CLI with explicit arguments (no shell injection)
        Self::run_nomad_exec(alloc_id, &cmd_refs, &self.client.addr, &self.client.token)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ref job_id) = self.job_id {
            let path = format!("/v1/job/{}?purge=true", job_id);
            let _ = self.client.delete(&path).await;
        }

        self.running = false;
        self.job_id = None;
        self.alloc_id = None;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Nomad
    }

    fn is_running(&self) -> bool {
        self.running
    }

    async fn write_file_unchecked(&mut self, path: &str, content: &[u8]) -> Result<()> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);

        // Create parent directory first
        if let Some(parent) = std::path::Path::new(path).parent() {
            let parent_str = parent.to_string_lossy();
            if parent_str != "/" {
                let mkdir_cmd = format!("mkdir -p '{}'", parent_str);
                self.exec(&["sh", "-c", &mkdir_cmd]).await?;
            }
        }

        // Write file via exec + base64
        let write_cmd = format!("echo '{}' | base64 -d > '{}'", encoded, path);
        let result = self.exec(&["sh", "-c", &write_cmd]).await?;

        if !result.is_success() {
            bail!("Failed to write file {}: {}", path, result.stderr);
        }

        Ok(())
    }

    async fn read_file_unchecked(&mut self, path: &str) -> Result<Vec<u8>> {
        // Try the Nomad FS API first for better performance
        let alloc_id = self
            .alloc_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Nomad allocation not started"))?;

        let fs_path = format!("/v1/client/fs/cat/{}?path={}", alloc_id, path);
        match self.client.get(&fs_path).await {
            Ok(val) => {
                // FS API returns raw content
                if let Some(s) = val.as_str() {
                    return Ok(s.as_bytes().to_vec());
                }
                // Fall through to exec-based read
            }
            Err(_) => {
                // Fall through to exec-based read
            }
        }

        // Fallback: read via exec + base64
        let read_cmd = format!("base64 '{}'", path);
        let result = self.exec(&["sh", "-c", &read_cmd]).await?;

        if !result.is_success() {
            bail!("Failed to read file {}: {}", path, result.stderr);
        }

        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(result.stdout.trim())
            .context("Failed to decode base64 file content")?;

        Ok(decoded)
    }

    async fn remove_file_unchecked(&mut self, path: &str) -> Result<()> {
        let rm_cmd = format!("rm -f '{}'", path);
        self.exec(&["sh", "-c", &rm_cmd]).await?;
        Ok(())
    }

    async fn mkdir_unchecked(&mut self, path: &str, recursive: bool) -> Result<()> {
        let flag = if recursive { "-p" } else { "" };
        let cmd = format!("mkdir {} '{}'", flag, path);
        self.exec(&["sh", "-c", &cmd]).await?;
        Ok(())
    }

    async fn attach(&mut self, shell: Option<&str>) -> Result<i32> {
        let alloc_id = self
            .alloc_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Nomad allocation not started"))?;

        let shell = shell.unwrap_or("/bin/sh");

        // Use nomad alloc exec with -t for TTY, inheriting stdio for interactive use
        let mut command = Command::new("nomad");
        command.arg("alloc").arg("exec").arg("-t").arg(alloc_id);

        command.env("NOMAD_ADDR", &self.client.addr);
        if let Some(ref token) = self.client.token {
            command.env("NOMAD_TOKEN", token);
        }

        command.arg("--").arg(shell);

        // Run interactively (inherit stdio)
        command
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        let status = command
            .status()
            .context("Failed to attach to Nomad allocation")?;

        Ok(status.code().unwrap_or(1))
    }

    async fn inject_files(&mut self, files: &[super::FileInjection]) -> Result<()> {
        for file in files {
            if let Some(parent) = std::path::Path::new(&file.dest).parent() {
                let parent_str = parent.to_string_lossy();
                if parent_str != "/" {
                    self.mkdir(&parent_str, true).await?;
                }
            }
            self.write_file(&file.dest, &file.content).await?;
        }
        Ok(())
    }
}
