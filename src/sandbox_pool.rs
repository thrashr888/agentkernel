//! Generic sandbox pool for fast sandbox acquisition across all backends.
//!
//! This pool works with any backend that implements the Sandbox trait,
//! providing pre-warmed sandboxes for immediate use. This eliminates
//! container/VM start time for most operations.
//!
//! Supports: Docker, Podman, Apple Containers, Firecracker, Hyperlight

#![allow(dead_code)]

use anyhow::Result;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Duration, interval};

use crate::backend::{BackendType, ExecResult, Sandbox, SandboxConfig, create_sandbox};

/// Default pool configuration
const DEFAULT_POOL_SIZE: usize = 5;
const DEFAULT_MAX_POOL_SIZE: usize = 20;
const GC_INTERVAL_MS: u64 = 1000;
const GC_BATCH_SIZE: usize = 5;

/// A pooled sandbox ready for use
pub struct PooledSandbox {
    /// Unique identifier
    pub id: String,
    /// The underlying sandbox
    sandbox: Box<dyn Sandbox>,
    /// Backend type
    backend_type: BackendType,
}

impl std::fmt::Debug for PooledSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledSandbox")
            .field("id", &self.id)
            .field("backend_type", &self.backend_type)
            .field("is_running", &self.sandbox.is_running())
            .finish()
    }
}

impl PooledSandbox {
    /// Run a command in this sandbox
    pub async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        self.sandbox.exec(cmd).await
    }

    /// Check if sandbox is still running
    pub fn is_running(&self) -> bool {
        self.sandbox.is_running()
    }

    /// Get the backend type
    pub fn backend_type(&self) -> BackendType {
        self.backend_type
    }
}

/// Generic sandbox pool that works with any backend
pub struct SandboxPool {
    /// Pre-warmed sandboxes ready for use
    warm_pool: Arc<Mutex<VecDeque<PooledSandbox>>>,
    /// Sandboxes queued for async cleanup
    cleanup_queue: Arc<Mutex<VecDeque<Box<dyn Sandbox>>>>,
    /// Semaphore to limit concurrent sandbox starts
    start_semaphore: Arc<Semaphore>,
    /// Counter for unique sandbox names
    name_counter: AtomicUsize,
    /// Backend type to use
    backend_type: BackendType,
    /// Configuration for sandboxes
    config: SandboxConfig,
    /// Target pool size
    target_size: usize,
    /// Maximum pool size
    max_size: usize,
    /// Whether the pool is running
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl SandboxPool {
    /// Create a new sandbox pool for the specified backend
    pub fn new(backend_type: BackendType) -> Result<Self> {
        Self::with_config(
            backend_type,
            SandboxConfig::default(),
            DEFAULT_POOL_SIZE,
            DEFAULT_MAX_POOL_SIZE,
        )
    }

    /// Create a pool with custom settings
    pub fn with_config(
        backend_type: BackendType,
        config: SandboxConfig,
        target_size: usize,
        max_size: usize,
    ) -> Result<Self> {
        Ok(Self {
            warm_pool: Arc::new(Mutex::new(VecDeque::new())),
            cleanup_queue: Arc::new(Mutex::new(VecDeque::new())),
            start_semaphore: Arc::new(Semaphore::new(5)), // Max 5 concurrent starts
            name_counter: AtomicUsize::new(0),
            backend_type,
            config,
            target_size,
            max_size,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Start the pool (pre-warm sandboxes and start GC task)
    pub async fn start(&self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        // Pre-warm the pool
        self.warm_pool_to_target().await?;

        // Start background GC task
        self.spawn_gc_task();

        Ok(())
    }

    /// Stop the pool and clean up all sandboxes
    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        // Drain warm pool to cleanup queue
        {
            let mut warm = self.warm_pool.lock().await;
            let mut cleanup = self.cleanup_queue.lock().await;
            while let Some(pooled) = warm.pop_front() {
                cleanup.push_back(pooled.sandbox);
            }
        }

        // Force immediate GC
        self.gc_all().await;

        Ok(())
    }

    /// Acquire a sandbox from the pool
    /// Returns immediately if pool has sandboxes, otherwise creates one
    pub async fn acquire(&self) -> Result<PooledSandbox> {
        // Try to get from warm pool first
        {
            let mut pool = self.warm_pool.lock().await;
            // Find a running sandbox
            while let Some(sandbox) = pool.pop_front() {
                if sandbox.is_running() {
                    // Trigger async refill
                    self.spawn_refill_task();
                    return Ok(sandbox);
                }
                // Sandbox died, skip it
            }
        }

        // Pool empty or all dead, create a new sandbox
        self.create_sandbox().await
    }

    /// Release a sandbox back to the pool or queue for cleanup
    pub async fn release(&self, sandbox: PooledSandbox) {
        if !sandbox.is_running() {
            // Dead sandbox, queue for cleanup
            let mut cleanup = self.cleanup_queue.lock().await;
            cleanup.push_back(sandbox.sandbox);
            return;
        }

        let pool_size = {
            let pool = self.warm_pool.lock().await;
            pool.len()
        };

        if pool_size < self.max_size {
            // Return to pool for reuse
            let mut pool = self.warm_pool.lock().await;
            pool.push_back(sandbox);
        } else {
            // Pool full, queue for cleanup
            let mut cleanup = self.cleanup_queue.lock().await;
            cleanup.push_back(sandbox.sandbox);
        }
    }

    /// Get current pool statistics
    pub async fn stats(&self) -> SandboxPoolStats {
        let warm = self.warm_pool.lock().await;
        let cleanup = self.cleanup_queue.lock().await;
        SandboxPoolStats {
            warm_count: warm.len(),
            cleanup_pending: cleanup.len(),
            target_size: self.target_size,
            max_size: self.max_size,
            backend_type: self.backend_type,
        }
    }

    /// Create a new sandbox
    async fn create_sandbox(&self) -> Result<PooledSandbox> {
        let _permit = self.start_semaphore.acquire().await?;

        let id = self.name_counter.fetch_add(1, Ordering::SeqCst);
        let name = format!("pool-{}-{}", self.backend_type, id);

        let mut sandbox = create_sandbox(self.backend_type, &name)?;
        sandbox.start(&self.config).await?;

        Ok(PooledSandbox {
            id: name,
            sandbox,
            backend_type: self.backend_type,
        })
    }

    /// Warm the pool up to target size
    async fn warm_pool_to_target(&self) -> Result<()> {
        let current_size = {
            let pool = self.warm_pool.lock().await;
            pool.len()
        };

        let needed = self.target_size.saturating_sub(current_size);
        if needed == 0 {
            return Ok(());
        }

        eprintln!(
            "Warming {} pool: creating {} sandboxes...",
            self.backend_type, needed
        );

        // Create sandboxes sequentially to avoid overwhelming the system
        let mut created = 0;
        for _ in 0..needed {
            match self.create_sandbox().await {
                Ok(sandbox) => {
                    let mut pool = self.warm_pool.lock().await;
                    pool.push_back(sandbox);
                    created += 1;
                }
                Err(e) => {
                    eprintln!("Warning: Failed to create sandbox: {}", e);
                }
            }
        }

        eprintln!(
            "{} pool warmed: {} sandboxes ready",
            self.backend_type, created
        );
        Ok(())
    }

    /// Spawn a background task to refill the pool
    fn spawn_refill_task(&self) {
        let warm_pool = Arc::clone(&self.warm_pool);
        let start_semaphore = Arc::clone(&self.start_semaphore);
        let running = Arc::clone(&self.running);
        let backend_type = self.backend_type;
        let config = self.config.clone();
        let target_size = self.target_size;
        let name_counter = self.name_counter.fetch_add(1, Ordering::SeqCst);

        tokio::spawn(async move {
            if !running.load(Ordering::SeqCst) {
                return;
            }

            let current_size = {
                let pool = warm_pool.lock().await;
                pool.len()
            };

            if current_size >= target_size {
                return;
            }

            // Create one sandbox to refill
            if let Ok(_permit) = start_semaphore.try_acquire() {
                let name = format!("pool-{}-{}", backend_type, name_counter);
                if let Ok(mut sandbox) = create_sandbox(backend_type, &name)
                    && sandbox.start(&config).await.is_ok()
                {
                    let pooled = PooledSandbox {
                        id: name,
                        sandbox,
                        backend_type,
                    };
                    let mut pool = warm_pool.lock().await;
                    pool.push_back(pooled);
                }
            }
        });
    }

    /// Spawn the background GC task
    fn spawn_gc_task(&self) {
        let cleanup_queue = Arc::clone(&self.cleanup_queue);
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(GC_INTERVAL_MS));
            while running.load(Ordering::SeqCst) {
                interval.tick().await;

                // Clean up a batch of sandboxes
                let to_cleanup: Vec<Box<dyn Sandbox>> = {
                    let mut queue = cleanup_queue.lock().await;
                    let mut batch = Vec::new();
                    for _ in 0..GC_BATCH_SIZE {
                        if let Some(sandbox) = queue.pop_front() {
                            batch.push(sandbox);
                        } else {
                            break;
                        }
                    }
                    batch
                };

                for mut sandbox in to_cleanup {
                    let _ = sandbox.stop().await;
                }
            }
        });
    }

    /// Force garbage collection of all pending sandboxes
    async fn gc_all(&self) {
        loop {
            let to_cleanup: Vec<Box<dyn Sandbox>> = {
                let mut queue = self.cleanup_queue.lock().await;
                let mut batch = Vec::new();
                for _ in 0..GC_BATCH_SIZE {
                    if let Some(sandbox) = queue.pop_front() {
                        batch.push(sandbox);
                    } else {
                        break;
                    }
                }
                batch
            };

            if to_cleanup.is_empty() {
                break;
            }

            for mut sandbox in to_cleanup {
                let _ = sandbox.stop().await;
            }
        }
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct SandboxPoolStats {
    pub warm_count: usize,
    pub cleanup_pending: usize,
    pub target_size: usize,
    pub max_size: usize,
    pub backend_type: BackendType,
}

impl std::fmt::Display for SandboxPoolStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} pool: {}/{} warm, {} pending cleanup",
            self.backend_type, self.warm_count, self.target_size, self.cleanup_pending
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Docker or Apple containers
    async fn test_sandbox_pool_basic() {
        // Try to detect available backend
        let backend = crate::backend::detect_best_backend().expect("No backend available");

        let config = SandboxConfig::with_image("alpine:3.20");
        let pool = SandboxPool::with_config(backend, config, 2, 5).unwrap();
        pool.start().await.unwrap();

        // Acquire a sandbox
        let mut sandbox = pool.acquire().await.unwrap();
        assert!(sandbox.is_running());

        // Run a command
        let result = sandbox.exec(&["echo", "hello"]).await.unwrap();
        assert!(result.is_success());
        assert!(result.stdout.contains("hello"));

        // Release back to pool
        pool.release(sandbox).await;

        // Check stats
        let stats = pool.stats().await;
        assert!(stats.warm_count >= 1);

        pool.stop().await.unwrap();
    }
}
