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

use anyhow::{bail, Result};

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use anyhow::Context;

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use hyperlight_wasm::{LoadedWasmSandbox, SandboxBuilder};

/// Check if Hyperlight is available on this system
pub fn hyperlight_available() -> bool {
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        // Check if KVM is available and hypervisor is present
        std::path::Path::new("/dev/kvm").exists()
            && hyperlight_wasm::is_hypervisor_present().unwrap_or(false)
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
