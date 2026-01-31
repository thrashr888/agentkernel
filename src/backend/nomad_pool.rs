//! Nomad warm pool manager.
//!
//! Uses parameterized batch jobs to maintain a pool of idle allocations.
//! acquire() claims an idle allocation, release() stops it and dispatches
//! a replacement.
//!
//! Compile with `--features nomad` to enable.

#![cfg(feature = "nomad")]

use anyhow::{Context, Result, bail};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

/// HTTP client wrapper for Nomad pool operations
struct PoolNomadClient {
    addr: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl PoolNomadClient {
    fn new(addr: &str, token: Option<String>) -> Self {
        Self {
            addr: addr.to_string(),
            token,
            http: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.get(&url);
        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }
        let resp = req.send().await.context("Nomad API GET failed")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error: {}", body);
        }
        resp.json().await.context("Failed to parse Nomad response")
    }

    async fn put(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.put(&url).json(body);
        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }
        let resp = req.send().await.context("Nomad API PUT failed")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error: {}", body);
        }
        resp.json().await.context("Failed to parse Nomad response")
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.post(&url).json(body);
        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }
        let resp = req.send().await.context("Nomad API POST failed")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error: {}", body);
        }
        resp.json().await.context("Failed to parse Nomad response")
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.addr, path);
        let mut req = self.http.delete(&url);
        if let Some(ref token) = self.token {
            req = req.header("X-Nomad-Token", token);
        }
        let resp = req.send().await.context("Nomad API DELETE failed")?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Nomad API error: {}", body);
        }
        Ok(())
    }
}

/// Configuration for the Nomad warm pool
pub struct NomadPoolConfig {
    /// Nomad API address
    pub nomad_addr: String,
    /// Nomad ACL token
    pub nomad_token: Option<String>,
    /// Target warm pool size
    pub warm_pool_size: usize,
    /// Maximum total allocations
    pub max_pool_size: usize,
    /// Container image for warm allocations
    pub image: String,
    /// Nomad task driver
    pub driver: String,
    /// Nomad datacenter
    pub datacenter: String,
    /// Resource: memory MB
    pub memory_mb: u64,
    /// Resource: CPU MHz
    pub cpu_mhz: u32,
}

impl Default for NomadPoolConfig {
    fn default() -> Self {
        Self {
            nomad_addr: std::env::var("NOMAD_ADDR")
                .unwrap_or_else(|_| "http://127.0.0.1:4646".to_string()),
            nomad_token: std::env::var("NOMAD_TOKEN").ok(),
            warm_pool_size: 10,
            max_pool_size: 50,
            image: "alpine:3.20".to_string(),
            driver: "docker".to_string(),
            datacenter: "dc1".to_string(),
            memory_mb: 512,
            cpu_mhz: 1000,
        }
    }
}

/// Statistics about the Nomad warm pool
#[derive(Debug, Clone)]
pub struct NomadPoolStats {
    /// Number of idle (warm) allocations
    pub warm: usize,
    /// Number of active (claimed) allocations
    pub active: usize,
    /// Target warm size
    pub target_warm: usize,
    /// Maximum total
    pub max_total: usize,
}

/// The parameterized job ID used for the warm pool
const POOL_JOB_ID: &str = "agentkernel-warm-pool";

/// Manages a pool of pre-warmed Nomad allocations for fast sandbox acquisition.
///
/// Uses a parameterized batch job that can be dispatched to create new warm
/// allocations. Each allocation runs `sleep infinity` until claimed.
pub struct NomadPool {
    config: NomadPoolConfig,
    client: PoolNomadClient,
    /// Guard concurrent pool operations
    lock: Arc<Mutex<()>>,
}

impl NomadPool {
    /// Create a new pool manager
    pub fn new(config: NomadPoolConfig) -> Self {
        let client = PoolNomadClient::new(&config.nomad_addr, config.nomad_token.clone());
        Self {
            config,
            client,
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Register the parameterized batch job used for the warm pool
    async fn register_pool_job(&self) -> Result<()> {
        let driver_config = match self.config.driver.as_str() {
            "docker" => {
                json!({
                    "image": self.config.image,
                    "command": "sh",
                    "args": ["-c", "sleep infinity"],
                    "cap_drop": ["ALL"],
                    "privileged": false,
                })
            }
            _ => {
                json!({
                    "command": "sh",
                    "args": ["-c", "sleep infinity"],
                })
            }
        };

        let job_spec = json!({
            "Job": {
                "ID": POOL_JOB_ID,
                "Name": POOL_JOB_ID,
                "Type": "batch",
                "Datacenters": [self.config.datacenter],
                "Parameterized": {
                    "Payload": "forbidden",
                    "MetaRequired": [],
                    "MetaOptional": ["sandbox_name"]
                },
                "Meta": {
                    "agentkernel-managed": "true",
                    "agentkernel-pool": "warm"
                },
                "TaskGroups": [{
                    "Name": "sandbox",
                    "Count": 1,
                    "Tasks": [{
                        "Name": "sandbox",
                        "Driver": self.config.driver,
                        "Config": driver_config,
                        "Resources": {
                            "CPU": self.config.cpu_mhz,
                            "MemoryMB": self.config.memory_mb
                        },
                        "Meta": {
                            "agentkernel-pool": "warm"
                        }
                    }]
                }]
            }
        });

        self.client.put("/v1/jobs", &job_spec).await?;
        Ok(())
    }

    /// Dispatch a new warm allocation from the parameterized job
    async fn dispatch_warm(&self) -> Result<String> {
        let dispatch_body = json!({
            "Meta": {
                "sandbox_name": "",
                "agentkernel-pool-status": "warm"
            }
        });

        let result = self
            .client
            .post(&format!("/v1/job/{}/dispatch", POOL_JOB_ID), &dispatch_body)
            .await?;

        let dispatch_id = result
            .get("DispatchedJobID")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No DispatchedJobID in response"))?
            .to_string();

        Ok(dispatch_id)
    }

    /// Initialize the pool by registering the job and dispatching warm allocations
    pub async fn initialize(&self) -> Result<()> {
        let _guard = self.lock.lock().await;

        // Register the parameterized job
        self.register_pool_job().await?;

        // Count existing warm allocations
        let warm_count = self.count_warm_allocs().await?;
        let needed = self.config.warm_pool_size.saturating_sub(warm_count);

        // Dispatch warm allocations
        for _ in 0..needed {
            if let Err(e) = self.dispatch_warm().await {
                eprintln!("Warning: Failed to dispatch warm allocation: {}", e);
            }
        }

        Ok(())
    }

    /// Count allocations with warm pool status
    async fn count_warm_allocs(&self) -> Result<usize> {
        let jobs = self
            .client
            .get("/v1/jobs?prefix=agentkernel-warm-pool/dispatch")
            .await?;
        let mut count = 0;

        if let Some(jobs_arr) = jobs.as_array() {
            for job in jobs_arr {
                let job_id = job.get("ID").and_then(|v| v.as_str()).unwrap_or("");
                let status = job.get("Status").and_then(|v| v.as_str()).unwrap_or("");

                if status == "running" {
                    // Check if this allocation is still in "warm" state (unclaimed)
                    let meta = job.get("Meta").and_then(|m| m.as_object());
                    let pool_status = meta
                        .and_then(|m| m.get("agentkernel-pool-status"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if pool_status == "warm" || job_id.contains("dispatch") {
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// Acquire an idle allocation from the pool.
    ///
    /// Returns (dispatched_job_id, alloc_id) on success.
    pub async fn acquire(&self, sandbox_name: &str) -> Result<(String, String)> {
        let _guard = self.lock.lock().await;

        // Find a running dispatched job that hasn't been claimed
        let jobs = self
            .client
            .get("/v1/jobs?prefix=agentkernel-warm-pool/dispatch")
            .await?;

        if let Some(jobs_arr) = jobs.as_array() {
            for job in jobs_arr {
                let job_id = job.get("ID").and_then(|v| v.as_str()).unwrap_or("");
                let status = job.get("Status").and_then(|v| v.as_str()).unwrap_or("");

                if status != "running" || job_id.is_empty() {
                    continue;
                }

                // Get allocations for this dispatched job
                let allocs = self
                    .client
                    .get(&format!("/v1/job/{}/allocations", job_id))
                    .await?;

                if let Some(allocs_arr) = allocs.as_array() {
                    for alloc in allocs_arr {
                        let alloc_status = alloc
                            .get("ClientStatus")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let alloc_id = alloc.get("ID").and_then(|v| v.as_str()).unwrap_or("");

                        if alloc_status == "running" && !alloc_id.is_empty() {
                            // Found a warm allocation -- return it
                            // The caller will use `nomad alloc exec` on this alloc_id
                            return Ok((job_id.to_string(), alloc_id.to_string()));
                        }
                    }
                }
            }
        }

        bail!("No warm allocations available in pool")
    }

    /// Release an allocation (stop the dispatched job and replenish)
    pub async fn release(&self, dispatched_job_id: &str) -> Result<()> {
        // Stop and purge the dispatched job
        let path = format!("/v1/job/{}?purge=true", dispatched_job_id);
        let _ = self.client.delete(&path).await;

        // Replenish
        self.replenish().await?;

        Ok(())
    }

    /// Replenish the warm pool to the target size
    pub async fn replenish(&self) -> Result<()> {
        let warm_count = self.count_warm_allocs().await?;
        let needed = self.config.warm_pool_size.saturating_sub(warm_count);

        for _ in 0..needed {
            if let Err(e) = self.dispatch_warm().await {
                eprintln!("Warning: Failed to replenish warm allocation: {}", e);
            }
        }

        Ok(())
    }

    /// Get pool statistics
    pub async fn stats(&self) -> Result<NomadPoolStats> {
        let warm = self.count_warm_allocs().await?;

        // Count active (this is approximate since we track warm, not active)
        let all_jobs = self
            .client
            .get("/v1/jobs?prefix=agentkernel-warm-pool")
            .await?;
        let total_running = all_jobs
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|j| j.get("Status").and_then(|v| v.as_str()) == Some("running"))
                    .count()
            })
            .unwrap_or(0);

        Ok(NomadPoolStats {
            warm,
            active: total_running.saturating_sub(warm),
            target_warm: self.config.warm_pool_size,
            max_total: self.config.max_pool_size,
        })
    }

    /// Spawn a background replenish task
    pub fn spawn_replenish_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Err(e) = self.replenish().await {
                    eprintln!("Nomad warm pool replenish error: {}", e);
                }
            }
        })
    }

    /// Clean up all warm pool jobs (for shutdown)
    pub async fn cleanup(&self) -> Result<()> {
        let jobs = self
            .client
            .get("/v1/jobs?prefix=agentkernel-warm-pool")
            .await?;

        if let Some(jobs_arr) = jobs.as_array() {
            for job in jobs_arr {
                if let Some(job_id) = job.get("ID").and_then(|v| v.as_str()) {
                    let path = format!("/v1/job/{}?purge=true", job_id);
                    let _ = self.client.delete(&path).await;
                }
            }
        }

        Ok(())
    }
}
