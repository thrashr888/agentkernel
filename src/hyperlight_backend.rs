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

use anyhow::{Result, bail};

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use anyhow::Context;

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use hyperlight_host::{GuestBinary, MultiUseSandbox, UninitializedSandbox};

/// Check if Hyperlight is available on this system
pub fn hyperlight_available() -> bool {
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        // Check if KVM is available
        std::path::Path::new("/dev/kvm").exists()
    }

    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        false
    }
}

/// Hyperlight-based sandbox for sub-millisecond execution
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
pub struct HyperlightSandbox {
    name: String,
    sandbox: Option<MultiUseSandbox>,
}

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
impl HyperlightSandbox {
    /// Create a new Hyperlight sandbox
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            sandbox: None,
        }
    }

    /// Initialize the sandbox with a Wasm module
    pub fn init_with_wasm(&mut self, wasm_bytes: &[u8]) -> Result<()> {
        // Create uninitialized sandbox with Wasm binary
        let uninit = UninitializedSandbox::new(GuestBinary::Buffer(wasm_bytes), None)
            .context("Failed to create Hyperlight sandbox")?;

        // Evolve to multi-use sandbox
        let sandbox = uninit
            .evolve()
            .context("Failed to initialize Hyperlight sandbox")?;

        self.sandbox = Some(sandbox);
        Ok(())
    }

    /// Run a Wasm module and return its output
    ///
    /// The module should export a `_start` function (WASI convention) or
    /// a `main` function that will be called.
    pub async fn run_wasm(&self, _args: &[String]) -> Result<String> {
        let sandbox = self
            .sandbox
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sandbox not initialized"))?;

        // TODO: Execute the Wasm module
        // For now, this is a placeholder - actual implementation requires
        // setting up WASI imports and calling the module's entry point

        bail!("Wasm execution not yet implemented - see hyperlight-wasm for examples")
    }

    /// Execute a named function from the Wasm module
    pub async fn call_function<T: Default>(
        &self,
        _name: &str,
        _args: &[serde_json::Value],
    ) -> Result<T> {
        let _sandbox = self
            .sandbox
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sandbox not initialized"))?;

        // TODO: Call exported function by name
        bail!("Function calls not yet implemented")
    }

    /// Check if the sandbox is initialized
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

    pub async fn run_wasm(&self, _args: &[String]) -> Result<String> {
        bail!("Hyperlight is not available on this platform")
    }

    pub async fn call_function<T: Default>(
        &self,
        _name: &str,
        _args: &[serde_json::Value],
    ) -> Result<T> {
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
