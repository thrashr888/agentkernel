//! Container pool for fast sandbox acquisition.
//!
//! Instead of creating/destroying containers per command, we maintain a pool
//! of pre-warmed containers ready for immediate use. This eliminates the
//! ~150ms container start time for most operations.
//!
//! Architecture:
//! - Warm pool: Pre-started containers waiting for work
//! - Active set: Containers currently in use
//! - Cleanup queue: Containers pending async garbage collection
//! - Persistent exec: Keep shell sessions open for faster command execution

// Allow unused pool API methods - they're part of the public API for future use
#![allow(dead_code)]

use anyhow::{Result, bail};
use std::collections::VecDeque;
use std::io::Write;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Duration, interval};

use crate::docker_backend::{ContainerRuntime, ContainerSandbox, detect_container_runtime};
use crate::permissions::Permissions;

/// Sentinel marker for detecting end of command output
const OUTPUT_SENTINEL: &str = "___AGENTKERNEL_DONE___";

/// A persistent shell session for fast command execution
pub struct PersistentShell {
    child: Child,
    stdin: ChildStdin,
    container_name: String,
}

impl PersistentShell {
    /// Create a new persistent shell session in the container
    pub fn new(runtime: ContainerRuntime, container_name: &str) -> Result<Self> {
        let mut child = std::process::Command::new(runtime.cmd())
            .args(["exec", "-i", container_name, "sh"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdin"))?;

        Ok(Self {
            child,
            stdin,
            container_name: container_name.to_string(),
        })
    }

    /// Run a command through the persistent shell
    /// Uses a sentinel to detect end of output
    pub fn run_command(&mut self, cmd: &[String]) -> Result<String> {
        // Build the command with sentinel
        let cmd_str = cmd.join(" ");
        let full_cmd = format!("({}) 2>&1; echo '{}'\n", cmd_str, OUTPUT_SENTINEL);

        // Write command to stdin
        self.stdin.write_all(full_cmd.as_bytes())?;
        self.stdin.flush()?;

        // Read output until sentinel
        // Note: This is synchronous; for production use, we'd want async I/O
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdout"))?;

        let mut output = String::new();
        let mut buf = [0u8; 4096];

        use std::io::Read;
        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            output.push_str(&String::from_utf8_lossy(&buf[..n]));
            if output.contains(OUTPUT_SENTINEL) {
                break;
            }
        }

        // Remove sentinel from output
        if let Some(pos) = output.find(OUTPUT_SENTINEL) {
            output.truncate(pos);
        }

        // Trim trailing newline
        Ok(output.trim_end().to_string())
    }

    /// Check if the shell is still alive
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PersistentShell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Default pool configuration
const DEFAULT_POOL_SIZE: usize = 10;
const DEFAULT_MAX_POOL_SIZE: usize = 50;
const DEFAULT_IMAGE: &str = "alpine:3.20";
const GC_INTERVAL_MS: u64 = 1000;
const GC_BATCH_SIZE: usize = 10;

/// A pooled container ready for use
pub struct PooledContainer {
    pub name: String,
    #[allow(dead_code)] // Used for container lifecycle tracking
    pub container_id: String,
    runtime: ContainerRuntime,
    /// Persistent shell for faster command execution (optional)
    persistent_shell: Option<std::sync::Mutex<PersistentShell>>,
}

impl std::fmt::Debug for PooledContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledContainer")
            .field("name", &self.name)
            .field("container_id", &self.container_id)
            .field("runtime", &self.runtime)
            .field("persistent_shell", &self.persistent_shell.is_some())
            .finish()
    }
}

impl PooledContainer {
    /// Run a command in this container using the fastest available method
    pub async fn run_command(&self, cmd: &[String]) -> Result<String> {
        // Try persistent shell first (faster: ~15-20ms vs ~100ms for docker exec)
        if let Some(ref shell_mutex) = self.persistent_shell
            && let Ok(mut shell) = shell_mutex.lock()
            && shell.is_alive()
        {
            return shell.run_command(cmd);
        }

        // Fallback to docker exec
        self.run_command_exec(cmd).await
    }

    /// Run a command using docker exec (slower but more reliable)
    pub async fn run_command_exec(&self, cmd: &[String]) -> Result<String> {
        let runtime_cmd = self.runtime.cmd();
        let container_name = format!("agentkernel-{}", self.name);

        let mut args = vec!["exec", &container_name];
        let cmd_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        args.extend(cmd_refs);

        let output = std::process::Command::new(runtime_cmd)
            .args(&args)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Command failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Initialize the persistent shell for this container
    pub fn init_persistent_shell(&mut self) -> Result<()> {
        let container_name = format!("agentkernel-{}", self.name);
        let shell = PersistentShell::new(self.runtime, &container_name)?;
        self.persistent_shell = Some(std::sync::Mutex::new(shell));
        Ok(())
    }
}

/// Container pool manager
pub struct ContainerPool {
    /// Pre-warmed containers ready for use
    warm_pool: Arc<Mutex<VecDeque<PooledContainer>>>,
    /// Containers queued for async cleanup
    cleanup_queue: Arc<Mutex<VecDeque<String>>>,
    /// Semaphore to limit concurrent container starts
    start_semaphore: Arc<Semaphore>,
    /// Counter for unique container names
    name_counter: AtomicUsize,
    /// Container runtime to use
    runtime: ContainerRuntime,
    /// Image to use for pooled containers
    image: String,
    /// Target pool size
    target_size: usize,
    /// Maximum pool size
    max_size: usize,
    /// Whether the pool is running
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl ContainerPool {
    /// Create a new container pool
    pub fn new() -> Result<Self> {
        let runtime = detect_container_runtime()
            .ok_or_else(|| anyhow::anyhow!("No container runtime available"))?;

        Ok(Self {
            warm_pool: Arc::new(Mutex::new(VecDeque::new())),
            cleanup_queue: Arc::new(Mutex::new(VecDeque::new())),
            start_semaphore: Arc::new(Semaphore::new(10)), // Max 10 concurrent starts
            name_counter: AtomicUsize::new(0),
            runtime,
            image: DEFAULT_IMAGE.to_string(),
            target_size: DEFAULT_POOL_SIZE,
            max_size: DEFAULT_MAX_POOL_SIZE,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Create a pool with custom settings
    pub fn with_config(target_size: usize, max_size: usize, image: &str) -> Result<Self> {
        let mut pool = Self::new()?;
        pool.target_size = target_size;
        pool.max_size = max_size;
        pool.image = image.to_string();
        Ok(pool)
    }

    /// Start the pool (pre-warm containers and start GC task)
    pub async fn start(&self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        // Pre-warm the pool
        self.warm_pool_to_target().await?;

        // Start background GC task
        self.spawn_gc_task();

        Ok(())
    }

    /// Stop the pool and clean up all containers
    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);

        // Drain warm pool to cleanup queue
        {
            let mut warm = self.warm_pool.lock().await;
            let mut cleanup = self.cleanup_queue.lock().await;
            while let Some(container) = warm.pop_front() {
                cleanup.push_back(container.name);
            }
        }

        // Force immediate GC
        self.gc_all().await;

        Ok(())
    }

    /// Acquire a container from the pool
    /// Returns immediately if pool has containers, otherwise creates one
    pub async fn acquire(&self) -> Result<PooledContainer> {
        // Try to get from warm pool first
        {
            let mut pool = self.warm_pool.lock().await;
            if let Some(container) = pool.pop_front() {
                // Trigger async refill
                self.spawn_refill_task();
                return Ok(container);
            }
        }

        // Pool empty, create a new container
        self.create_container().await
    }

    /// Release a container back to the pool or queue for cleanup
    pub async fn release(&self, container: PooledContainer) {
        let pool_size = {
            let pool = self.warm_pool.lock().await;
            pool.len()
        };

        if pool_size < self.max_size {
            // Return to pool for reuse
            let mut pool = self.warm_pool.lock().await;
            pool.push_back(container);
        } else {
            // Pool full, queue for cleanup
            let mut cleanup = self.cleanup_queue.lock().await;
            cleanup.push_back(container.name);
        }
    }

    /// Release a container by name (queues for cleanup, doesn't return to pool)
    pub async fn release_for_cleanup(&self, name: String) {
        let mut cleanup = self.cleanup_queue.lock().await;
        cleanup.push_back(name);
    }

    /// Get current pool statistics
    pub async fn stats(&self) -> PoolStats {
        let warm = self.warm_pool.lock().await;
        let cleanup = self.cleanup_queue.lock().await;
        PoolStats {
            warm_count: warm.len(),
            cleanup_pending: cleanup.len(),
            target_size: self.target_size,
            max_size: self.max_size,
        }
    }

    /// Create a new container
    async fn create_container(&self) -> Result<PooledContainer> {
        let _permit = self.start_semaphore.acquire().await?;

        let id = self.name_counter.fetch_add(1, Ordering::SeqCst);
        let name = format!("pool-{}", id);

        let mut sandbox = ContainerSandbox::with_runtime(&name, self.runtime);
        sandbox
            .start_with_permissions(&self.image, &Permissions::default())
            .await?;

        // Get container ID
        let container_name = format!("agentkernel-{}", name);
        let output = std::process::Command::new(self.runtime.cmd())
            .args(["inspect", "-f", "{{.Id}}", &container_name])
            .output()?;

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Create container with persistent shell for faster execution
        let mut container = PooledContainer {
            name,
            container_id,
            runtime: self.runtime,
            persistent_shell: None,
        };

        // Try to initialize persistent shell (non-fatal if it fails)
        if let Err(e) = container.init_persistent_shell() {
            eprintln!("Warning: Failed to init persistent shell: {}", e);
        }

        Ok(container)
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

        eprintln!("Warming pool: creating {} containers...", needed);

        // Create containers in parallel
        let mut handles = Vec::new();
        for _ in 0..needed {
            let pool = self.clone_for_task();
            handles.push(tokio::spawn(async move { pool.create_container().await }));
        }

        // Collect results
        let mut created = 0;
        for handle in handles {
            if let Ok(Ok(container)) = handle.await {
                let mut pool = self.warm_pool.lock().await;
                pool.push_back(container);
                created += 1;
            }
        }

        eprintln!("Pool warmed: {} containers ready", created);
        Ok(())
    }

    /// Spawn a background task to refill the pool
    fn spawn_refill_task(&self) {
        let pool = self.clone_for_task();
        tokio::spawn(async move {
            let _ = pool.warm_pool_to_target().await;
        });
    }

    /// Spawn the background GC task
    fn spawn_gc_task(&self) {
        let pool = self.clone_for_task();
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(GC_INTERVAL_MS));
            while pool.running.load(Ordering::SeqCst) {
                interval.tick().await;
                pool.gc_batch().await;
            }
        });
    }

    /// Run garbage collection on a batch of containers
    async fn gc_batch(&self) {
        let to_cleanup: Vec<String> = {
            let mut queue = self.cleanup_queue.lock().await;
            let mut batch = Vec::new();
            for _ in 0..GC_BATCH_SIZE {
                if let Some(name) = queue.pop_front() {
                    batch.push(name);
                } else {
                    break;
                }
            }
            batch
        };

        if to_cleanup.is_empty() {
            return;
        }

        // Clean up containers in parallel
        let runtime = self.runtime;
        let handles: Vec<_> = to_cleanup
            .into_iter()
            .map(|name| {
                let container_name = format!("agentkernel-{}", name);
                tokio::spawn(async move {
                    let _ = std::process::Command::new(runtime.cmd())
                        .args(["rm", "-f", &container_name])
                        .output();
                })
            })
            .collect();

        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Force garbage collection of all pending containers
    async fn gc_all(&self) {
        loop {
            let remaining = {
                let queue = self.cleanup_queue.lock().await;
                queue.len()
            };
            if remaining == 0 {
                break;
            }
            self.gc_batch().await;
        }
    }

    /// Clone references for use in spawned tasks
    fn clone_for_task(&self) -> ContainerPoolHandle {
        ContainerPoolHandle {
            warm_pool: Arc::clone(&self.warm_pool),
            cleanup_queue: Arc::clone(&self.cleanup_queue),
            start_semaphore: Arc::clone(&self.start_semaphore),
            name_counter: self.name_counter.load(Ordering::SeqCst),
            runtime: self.runtime,
            image: self.image.clone(),
            target_size: self.target_size,
            running: Arc::clone(&self.running),
        }
    }
}

/// Lightweight handle for pool operations in spawned tasks
struct ContainerPoolHandle {
    warm_pool: Arc<Mutex<VecDeque<PooledContainer>>>,
    cleanup_queue: Arc<Mutex<VecDeque<String>>>,
    start_semaphore: Arc<Semaphore>,
    name_counter: usize,
    runtime: ContainerRuntime,
    image: String,
    target_size: usize,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl ContainerPoolHandle {
    async fn warm_pool_to_target(&self) -> Result<()> {
        let current_size = {
            let pool = self.warm_pool.lock().await;
            pool.len()
        };

        let needed = self.target_size.saturating_sub(current_size);
        if needed == 0 {
            return Ok(());
        }

        // Create containers
        for i in 0..needed {
            let _permit = self.start_semaphore.acquire().await?;

            let name = format!("pool-{}", self.name_counter + i);
            let mut sandbox = ContainerSandbox::with_runtime(&name, self.runtime);
            if sandbox
                .start_with_permissions(&self.image, &Permissions::default())
                .await
                .is_ok()
            {
                let container_name = format!("agentkernel-{}", name);
                if let Ok(output) = std::process::Command::new(self.runtime.cmd())
                    .args(["inspect", "-f", "{{.Id}}", &container_name])
                    .output()
                {
                    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let mut container = PooledContainer {
                        name,
                        container_id,
                        runtime: self.runtime,
                        persistent_shell: None,
                    };
                    // Try to init persistent shell
                    let _ = container.init_persistent_shell();
                    let mut pool = self.warm_pool.lock().await;
                    pool.push_back(container);
                }
            }
        }

        Ok(())
    }

    async fn gc_batch(&self) {
        let to_cleanup: Vec<String> = {
            let mut queue = self.cleanup_queue.lock().await;
            let mut batch = Vec::new();
            for _ in 0..GC_BATCH_SIZE {
                if let Some(name) = queue.pop_front() {
                    batch.push(name);
                } else {
                    break;
                }
            }
            batch
        };

        if to_cleanup.is_empty() {
            return;
        }

        let runtime = self.runtime;
        for name in to_cleanup {
            let container_name = format!("agentkernel-{}", name);
            let _ = std::process::Command::new(runtime.cmd())
                .args(["rm", "-f", &container_name])
                .output();
        }
    }

    async fn create_container(&self) -> Result<PooledContainer> {
        let _permit = self.start_semaphore.acquire().await?;

        // Use a unique timestamp-based name to avoid conflicts
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("pool-{}", id);

        let mut sandbox = ContainerSandbox::with_runtime(&name, self.runtime);
        sandbox
            .start_with_permissions(&self.image, &Permissions::default())
            .await?;

        let container_name = format!("agentkernel-{}", name);
        let output = std::process::Command::new(self.runtime.cmd())
            .args(["inspect", "-f", "{{.Id}}", &container_name])
            .output()?;

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let mut container = PooledContainer {
            name,
            container_id,
            runtime: self.runtime,
            persistent_shell: None,
        };

        // Try to init persistent shell
        let _ = container.init_persistent_shell();

        Ok(container)
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub warm_count: usize,
    pub cleanup_pending: usize,
    pub target_size: usize,
    pub max_size: usize,
}

impl std::fmt::Display for PoolStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pool: {}/{} warm, {} pending cleanup",
            self.warm_count, self.target_size, self.cleanup_pending
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === PoolStats tests ===

    #[test]
    fn test_pool_stats_display() {
        let stats = PoolStats {
            warm_count: 5,
            cleanup_pending: 2,
            target_size: 10,
            max_size: 50,
        };
        let display = format!("{}", stats);
        assert!(display.contains("5/10 warm"));
        assert!(display.contains("2 pending cleanup"));
    }

    #[test]
    fn test_pool_stats_display_zero() {
        let stats = PoolStats {
            warm_count: 0,
            cleanup_pending: 0,
            target_size: 5,
            max_size: 20,
        };
        let display = format!("{}", stats);
        assert!(display.contains("0/5 warm"));
        assert!(display.contains("0 pending cleanup"));
    }

    #[test]
    fn test_pool_stats_debug() {
        let stats = PoolStats {
            warm_count: 3,
            cleanup_pending: 1,
            target_size: 5,
            max_size: 10,
        };
        let debug = format!("{:?}", stats);
        assert!(debug.contains("warm_count: 3"));
        assert!(debug.contains("cleanup_pending: 1"));
        assert!(debug.contains("target_size: 5"));
        assert!(debug.contains("max_size: 10"));
    }

    #[test]
    fn test_pool_stats_clone() {
        let stats = PoolStats {
            warm_count: 5,
            cleanup_pending: 2,
            target_size: 10,
            max_size: 50,
        };
        let cloned = stats.clone();
        assert_eq!(cloned.warm_count, 5);
        assert_eq!(cloned.cleanup_pending, 2);
        assert_eq!(cloned.target_size, 10);
        assert_eq!(cloned.max_size, 50);
    }

    // === Constants tests ===

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_POOL_SIZE, 10);
        assert_eq!(DEFAULT_MAX_POOL_SIZE, 50);
        assert_eq!(DEFAULT_IMAGE, "alpine:3.20");
        assert_eq!(GC_INTERVAL_MS, 1000);
        assert_eq!(GC_BATCH_SIZE, 10);
    }

    #[test]
    fn test_output_sentinel_is_unique() {
        // Sentinel should be unique enough to not appear in normal output
        assert!(OUTPUT_SENTINEL.starts_with("___"));
        assert!(OUTPUT_SENTINEL.ends_with("___"));
        assert!(OUTPUT_SENTINEL.contains("AGENTKERNEL"));
    }

    // === ContainerPool construction tests (without starting) ===

    #[test]
    fn test_container_pool_with_config_values() {
        // Note: This test will fail if no container runtime is available,
        // which is expected in CI without Docker
        if detect_container_runtime().is_none() {
            eprintln!("Skipping test: No container runtime available");
            return;
        }

        let pool = ContainerPool::with_config(3, 15, "python:3.12-alpine").unwrap();
        assert_eq!(pool.target_size, 3);
        assert_eq!(pool.max_size, 15);
        assert_eq!(pool.image, "python:3.12-alpine");
    }

    #[test]
    fn test_container_pool_default_values() {
        if detect_container_runtime().is_none() {
            eprintln!("Skipping test: No container runtime available");
            return;
        }

        let pool = ContainerPool::new().unwrap();
        assert_eq!(pool.target_size, DEFAULT_POOL_SIZE);
        assert_eq!(pool.max_size, DEFAULT_MAX_POOL_SIZE);
        assert_eq!(pool.image, DEFAULT_IMAGE);
    }

    // === Integration test (requires Docker) ===

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_pool_basic() {
        let pool = ContainerPool::with_config(2, 5, "alpine:3.20").unwrap();
        pool.start().await.unwrap();

        // Acquire a container
        let container = pool.acquire().await.unwrap();
        assert!(!container.name.is_empty());

        // Run a command
        let output = container
            .run_command(&["echo".into(), "hello".into()])
            .await
            .unwrap();
        assert!(output.contains("hello"));

        // Release back to pool
        pool.release(container).await;

        // Check stats
        let stats = pool.stats().await;
        assert!(stats.warm_count >= 1);

        pool.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_pool_acquire_release_cycle() {
        let pool = ContainerPool::with_config(2, 5, "alpine:3.20").unwrap();
        pool.start().await.unwrap();

        // Acquire and release multiple times
        for i in 0..3 {
            let container = pool.acquire().await.unwrap();
            let output = container
                .run_command(&["echo".into(), format!("iteration-{}", i)])
                .await
                .unwrap();
            assert!(output.contains(&format!("iteration-{}", i)));
            pool.release(container).await;
        }

        pool.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_pool_stats_after_operations() {
        let pool = ContainerPool::with_config(2, 5, "alpine:3.20").unwrap();
        pool.start().await.unwrap();

        // Initial stats
        let initial = pool.stats().await;
        assert_eq!(initial.target_size, 2);
        assert_eq!(initial.max_size, 5);

        // Acquire should reduce warm count
        let container = pool.acquire().await.unwrap();

        // Release should increase it back
        pool.release(container).await;
        let after_release = pool.stats().await;
        assert!(after_release.warm_count >= 1);

        pool.stop().await.unwrap();
    }
}
