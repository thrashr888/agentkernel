//! Hyperlight WebAssembly backend implementing the Sandbox trait.
//!
//! Note: Hyperlight runs WebAssembly modules, not shell commands.
//! The `exec` method expects Wasm function names, not shell commands.

use anyhow::{Result, bail};
use async_trait::async_trait;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};

/// Check if Hyperlight is available on this system
pub fn hyperlight_available() -> bool {
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        std::path::Path::new("/dev/kvm").exists() && hyperlight_wasm::is_hypervisor_present()
    }

    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        false
    }
}

/// Hyperlight WebAssembly sandbox
///
/// Unlike other backends, Hyperlight runs WebAssembly modules directly
/// in a hypervisor-isolated micro VM. Commands are Wasm function names.
pub struct HyperlightSandbox {
    name: String,
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    sandbox: Option<hyperlight_wasm::LoadedWasmSandbox>,
    running: bool,
}

impl HyperlightSandbox {
    /// Create a new Hyperlight sandbox
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            #[cfg(all(target_os = "linux", feature = "hyperlight"))]
            sandbox: None,
            running: false,
        }
    }

    /// Initialize with a Wasm module (required before running functions)
    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    pub fn init_with_wasm(&mut self, wasm_bytes: &[u8]) -> Result<()> {
        use anyhow::Context;
        use hyperlight_wasm::SandboxBuilder;

        let proto = SandboxBuilder::new()
            .with_guest_heap_size(10_000_000)
            .with_guest_stack_size(1_000_000)
            .build()
            .context("Failed to build Hyperlight sandbox")?;

        let wasm_sandbox = proto
            .load_runtime()
            .context("Failed to load Hyperlight runtime")?;

        let loaded = wasm_sandbox
            .load_module_from_buffer(wasm_bytes)
            .context("Failed to load Wasm module")?;

        self.sandbox = Some(loaded);
        self.running = true;
        Ok(())
    }

    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    pub fn init_with_wasm(&mut self, _wasm_bytes: &[u8]) -> Result<()> {
        bail!("Hyperlight is not available on this platform")
    }
}

#[async_trait]
impl Sandbox for HyperlightSandbox {
    async fn start(&mut self, _config: &SandboxConfig) -> Result<()> {
        // Hyperlight requires a Wasm module to be loaded via init_with_wasm()
        // The SandboxConfig.image field could be used as a path to a .wasm file

        #[cfg(all(target_os = "linux", feature = "hyperlight"))]
        {
            // For now, just mark as "ready" - actual initialization requires wasm bytes
            // In a full implementation, we could load the wasm from config.image path
            self.running = true;
            Ok(())
        }

        #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
        {
            bail!("Hyperlight is not available on this platform. Requires Linux with KVM.")
        }
    }

    async fn exec(&mut self, cmd: &[&str]) -> Result<ExecResult> {
        #[cfg(all(target_os = "linux", feature = "hyperlight"))]
        {
            let sandbox = self
                .sandbox
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Sandbox not initialized with Wasm module"))?;

            // The first argument is the function name to call
            let func_name = cmd
                .first()
                .ok_or_else(|| anyhow::anyhow!("No function name provided"))?;

            // Call the Wasm function (no arguments for now)
            match sandbox.call_guest_function::<i32>(func_name, ()) {
                Ok(result) => Ok(ExecResult::success(result.to_string())),
                Err(e) => Ok(ExecResult::failure(1, e.to_string())),
            }
        }

        #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
        {
            let _ = cmd;
            bail!("Hyperlight is not available on this platform")
        }
    }

    async fn stop(&mut self) -> Result<()> {
        #[cfg(all(target_os = "linux", feature = "hyperlight"))]
        {
            self.sandbox = None;
        }
        self.running = false;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Hyperlight
    }

    fn is_running(&self) -> bool {
        #[cfg(all(target_os = "linux", feature = "hyperlight"))]
        {
            self.running && self.sandbox.is_some()
        }

        #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
        {
            false
        }
    }
}
