//! Hyperlight WebAssembly backend for sub-millisecond sandbox execution.
//!
//! This backend uses Microsoft's Hyperlight to run WebAssembly modules in
//! hypervisor-isolated micro VMs with ~1-2ms startup times.
//!
//! **Requirements:**
//! - Linux with KVM (`/dev/kvm` accessible)
//! - Feature flag: `--features hyperlight`
//!
//! **Platform support:**
//! - Linux (KVM): ✅ Supported
//! - Windows (WHP): ✅ Supported (not yet implemented here)
//! - macOS: ❌ Not supported (use Apple Containers backend)

#![allow(dead_code)]

use anyhow::Result;
#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
use anyhow::bail;

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use anyhow::Context;

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use hyperlight_wasm::{LoadedWasmSandbox, SandboxBuilder, WasmSandbox};

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use std::collections::VecDeque;
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use std::sync::{Arc, Mutex};
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use std::time::Instant;

/// Check if Hyperlight is available on this system
pub fn hyperlight_available() -> bool {
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        // Check if KVM is available and hypervisor is present
        std::path::Path::new("/dev/kvm").exists() && hyperlight_wasm::is_hypervisor_present()
    }

    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        false
    }
}

/// Hyperlight-based sandbox for sub-millisecond WebAssembly execution
///
/// Lifecycle:
/// 1. `new()` - Create sandbox instance
/// 2. `init_with_wasm()` - Load Wasm module (this actually creates the VM)
/// 3. `call_function()` - Execute exported functions
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
pub struct HyperlightSandbox {
    name: String,
    sandbox: Option<LoadedWasmSandbox>,
}

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
impl HyperlightSandbox {
    /// Create a new Hyperlight sandbox (does not start VM yet)
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            sandbox: None,
        }
    }

    /// Initialize the sandbox with a Wasm module
    ///
    /// This is the expensive operation that creates the VM and loads the module.
    /// Target: <2ms for module load + VM creation
    pub fn init_with_wasm(&mut self, wasm_bytes: &[u8]) -> Result<()> {
        // Build the sandbox configuration
        let proto = SandboxBuilder::new()
            .with_guest_heap_size(10_000_000) // 10MB heap
            .with_guest_stack_size(1_000_000) // 1MB stack
            .build()
            .context("Failed to build Hyperlight sandbox")?;

        // Load the Wasm runtime (this creates the micro VM)
        let wasm_sandbox = proto
            .load_runtime()
            .context("Failed to load Hyperlight runtime")?;

        // Load the Wasm module
        let loaded = wasm_sandbox
            .load_module_from_buffer(wasm_bytes)
            .context("Failed to load Wasm module")?;

        self.sandbox = Some(loaded);
        Ok(())
    }

    /// Call an exported function from the loaded Wasm module
    ///
    /// For WASI modules, call "_start" with no arguments.
    pub fn call_function<Output: hyperlight_wasm::SupportedReturnType>(
        &mut self,
        name: &str,
    ) -> Result<Output> {
        let sandbox = self
            .sandbox
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Sandbox not initialized"))?;

        sandbox
            .call_guest_function::<Output>(name, ())
            .context("Failed to call guest function")
    }

    /// Run a WASI module's _start function
    pub fn run_wasi(&mut self) -> Result<i32> {
        self.call_function::<i32>("_start")
    }

    /// Check if the sandbox is initialized with a module
    pub fn is_initialized(&self) -> bool {
        self.sandbox.is_some()
    }

    /// Get the sandbox name
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Pool configuration
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
#[derive(Debug, Clone)]
pub struct HyperlightPoolConfig {
    /// Minimum number of warm sandboxes to maintain
    pub min_warm: usize,
    /// Maximum number of warm sandboxes to maintain
    pub max_warm: usize,
    /// Guest heap size in bytes
    pub guest_heap_size: u64,
    /// Guest stack size in bytes
    pub guest_stack_size: u64,
}

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
impl Default for HyperlightPoolConfig {
    fn default() -> Self {
        Self {
            min_warm: 3,
            max_warm: 10,
            guest_heap_size: 10_000_000, // 10MB
            guest_stack_size: 1_000_000, // 1MB
        }
    }
}

/// A pre-warmed Hyperlight runtime ready for module loading
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
pub struct PooledRuntime {
    /// The warm WasmSandbox ready for module loading
    sandbox: WasmSandbox,
    /// When this runtime was created
    pub created_at: Instant,
}

/// Pool of pre-warmed Hyperlight runtimes for fast execution
///
/// The main cost of Hyperlight is runtime startup (~68ms). This pool
/// maintains pre-warmed runtimes so module loading and function calls
/// can happen in sub-millisecond time.
///
/// Usage:
/// ```ignore
/// let pool = HyperlightPool::new(HyperlightPoolConfig::default())?;
/// pool.warm_up()?;  // Pre-warm runtimes
///
/// // Fast path: acquire warm runtime, load module, execute
/// let runtime = pool.acquire()?;
/// let loaded = runtime.load_module_from_buffer(&wasm_bytes)?;
/// let result = loaded.call_guest_function::<i32>("main", ())?;
/// ```
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
pub struct HyperlightPool {
    /// Pre-warmed runtimes ready for use
    warm_pool: Arc<Mutex<VecDeque<PooledRuntime>>>,
    /// Pool configuration
    config: HyperlightPoolConfig,
    /// Counter for tracking pool operations
    acquired_count: AtomicUsize,
    /// Shutdown flag
    shutdown: AtomicBool,
}

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
impl HyperlightPool {
    /// Create a new Hyperlight pool
    pub fn new(config: HyperlightPoolConfig) -> Result<Self> {
        if !hyperlight_available() {
            anyhow::bail!("Hyperlight is not available on this system");
        }

        Ok(Self {
            warm_pool: Arc::new(Mutex::new(VecDeque::new())),
            config,
            acquired_count: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Create a pool with default configuration
    pub fn with_defaults() -> Result<Self> {
        Self::new(HyperlightPoolConfig::default())
    }

    /// Pre-warm the pool to min_warm runtimes
    pub fn warm_up(&self) -> Result<()> {
        let current = self.warm_pool.lock().unwrap().len();
        let needed = self.config.min_warm.saturating_sub(current);

        for _ in 0..needed {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            match self.create_runtime() {
                Ok(runtime) => {
                    self.warm_pool.lock().unwrap().push_back(runtime);
                }
                Err(e) => {
                    eprintln!("Failed to warm up Hyperlight runtime: {}", e);
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Acquire a warm runtime from the pool
    ///
    /// If the pool is empty, creates a new runtime (slower path).
    /// Returns the WasmSandbox ready for module loading.
    ///
    /// Note: This does NOT block on pool refill. Call `refill_if_needed()`
    /// separately if you want to maintain pool levels.
    pub fn acquire(&self) -> Result<WasmSandbox> {
        self.acquired_count.fetch_add(1, Ordering::SeqCst);

        // Try to get from warm pool first (fast path: just lock acquisition)
        {
            let mut pool = self.warm_pool.lock().unwrap();
            if let Some(runtime) = pool.pop_front() {
                return Ok(runtime.sandbox);
            }
        }

        // Pool empty, create a new runtime (slow path: ~68ms)
        let runtime = self.create_runtime()?;
        Ok(runtime.sandbox)
    }

    /// Refill the pool if below minimum (call this periodically or in background)
    pub fn refill_if_needed(&self) -> Result<()> {
        let current = self.warm_pool.lock().unwrap().len();
        if current < self.config.min_warm {
            self.refill_one()?;
        }
        Ok(())
    }

    /// Add one runtime to the pool (useful for pre-warming beyond min_warm)
    pub fn add_one(&self) -> Result<()> {
        let current = self.warm_pool.lock().unwrap().len();
        if current >= self.config.max_warm {
            return Ok(()); // Pool is full
        }
        let runtime = self.create_runtime()?;
        self.warm_pool.lock().unwrap().push_back(runtime);
        Ok(())
    }

    /// Pre-warm the pool to a specific count
    pub fn warm_to(&self, count: usize) -> Result<()> {
        let target = count.min(self.config.max_warm);
        let current = self.warm_pool.lock().unwrap().len();
        let needed = target.saturating_sub(current);

        for _ in 0..needed {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            match self.create_runtime() {
                Ok(runtime) => {
                    self.warm_pool.lock().unwrap().push_back(runtime);
                }
                Err(e) => {
                    eprintln!("Failed to warm up Hyperlight runtime: {}", e);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Get pool statistics
    pub fn stats(&self) -> HyperlightPoolStats {
        let warm_count = self.warm_pool.lock().unwrap().len();
        HyperlightPoolStats {
            warm_count,
            acquired_total: self.acquired_count.load(Ordering::SeqCst),
            config_min: self.config.min_warm,
            config_max: self.config.max_warm,
        }
    }

    /// Create a new warm runtime
    fn create_runtime(&self) -> Result<PooledRuntime> {
        let proto = SandboxBuilder::new()
            .with_guest_heap_size(self.config.guest_heap_size)
            .with_guest_stack_size(self.config.guest_stack_size)
            .build()
            .context("Failed to build Hyperlight sandbox")?;

        let sandbox = proto
            .load_runtime()
            .context("Failed to load Hyperlight runtime")?;

        Ok(PooledRuntime {
            sandbox,
            created_at: Instant::now(),
        })
    }

    /// Refill the pool with one runtime
    fn refill_one(&self) -> Result<()> {
        let current = self.warm_pool.lock().unwrap().len();
        if current >= self.config.max_warm {
            return Ok(()); // Pool is full
        }

        let runtime = self.create_runtime()?;
        self.warm_pool.lock().unwrap().push_back(runtime);
        Ok(())
    }

    /// Signal shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Clear all warm runtimes
    pub fn clear(&self) {
        let mut pool = self.warm_pool.lock().unwrap();
        pool.clear();
    }
}

/// Pool statistics
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
#[derive(Debug, Clone)]
pub struct HyperlightPoolStats {
    /// Number of warm runtimes ready
    pub warm_count: usize,
    /// Total number of acquires since pool creation
    pub acquired_total: usize,
    /// Configured minimum warm runtimes
    pub config_min: usize,
    /// Configured maximum warm runtimes
    pub config_max: usize,
}

// ============================================================================
// Stub implementations for non-Linux/non-hyperlight builds
// ============================================================================

/// Stub pool configuration
#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
#[derive(Debug, Clone, Default)]
pub struct HyperlightPoolConfig {
    pub min_warm: usize,
    pub max_warm: usize,
    pub guest_heap_size: u64,
    pub guest_stack_size: u64,
}

/// Stub pool statistics
#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
#[derive(Debug, Clone)]
pub struct HyperlightPoolStats {
    pub warm_count: usize,
    pub acquired_total: usize,
    pub config_min: usize,
    pub config_max: usize,
}

/// Stub pool implementation
#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
pub struct HyperlightPool;

#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
impl HyperlightPool {
    pub fn new(_config: HyperlightPoolConfig) -> Result<Self> {
        bail!("Hyperlight pool is not available on this platform")
    }

    pub fn with_defaults() -> Result<Self> {
        bail!("Hyperlight pool is not available on this platform")
    }

    pub fn warm_up(&self) -> Result<()> {
        bail!("Hyperlight pool is not available on this platform")
    }

    pub fn stats(&self) -> HyperlightPoolStats {
        HyperlightPoolStats {
            warm_count: 0,
            acquired_total: 0,
            config_min: 0,
            config_max: 0,
        }
    }

    pub fn shutdown(&self) {}
    pub fn clear(&self) {}
}

/// Stub implementation for non-Linux or non-hyperlight builds
#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
pub struct HyperlightSandbox {
    name: String,
}

#[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
impl HyperlightSandbox {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    pub fn init_with_wasm(&mut self, _wasm_bytes: &[u8]) -> Result<()> {
        bail!(
            "Hyperlight is not available on this platform. Requires Linux with KVM and --features hyperlight"
        )
    }

    pub fn call_function<T: Default>(&mut self, _name: &str) -> Result<T> {
        bail!("Hyperlight is not available on this platform")
    }

    pub fn run_wasi(&mut self) -> Result<i32> {
        bail!("Hyperlight is not available on this platform")
    }

    pub fn is_initialized(&self) -> bool {
        false
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyperlight_availability_check() {
        // Just verify the check doesn't panic
        let _ = hyperlight_available();
    }

    #[test]
    fn test_sandbox_creation() {
        let sandbox = HyperlightSandbox::new("test");
        assert_eq!(sandbox.name(), "test");
        assert!(!sandbox.is_initialized());
    }
}
