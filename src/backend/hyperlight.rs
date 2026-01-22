//! Hyperlight WebAssembly backend implementing the Sandbox trait.
//!
//! Note: Hyperlight runs WebAssembly modules, not shell commands.
//! The `exec` method expects Wasm function names, not shell commands.
//!
//! Supports both .wasm (binary) and .wat (text) format files.
//! WAT files are automatically compiled to WASM on load.

use anyhow::{Result, bail};
use async_trait::async_trait;
use std::path::Path;

use super::{BackendType, ExecResult, Sandbox, SandboxConfig};

/// Compile WAT (WebAssembly Text) to WASM binary
pub fn compile_wat(wat_source: &str) -> Result<Vec<u8>> {
    wat::parse_str(wat_source).map_err(|e| anyhow::anyhow!("WAT compilation error: {}", e))
}

/// Compile WAT file to WASM binary
pub fn compile_wat_file(path: &Path) -> Result<Vec<u8>> {
    let source = std::fs::read_to_string(path)?;
    compile_wat(&source)
}

/// Check if a file is WAT format (by extension)
pub fn is_wat_file(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.eq_ignore_ascii_case("wat"))
        .unwrap_or(false)
}

/// Load WASM from file, auto-detecting WAT format
pub fn load_wasm_file(path: &Path) -> Result<Vec<u8>> {
    if is_wat_file(path) {
        compile_wat_file(path)
    } else {
        std::fs::read(path).map_err(Into::into)
    }
}

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
    async fn start(&mut self, config: &SandboxConfig) -> Result<()> {
        // Hyperlight requires a Wasm module to be loaded
        // The SandboxConfig.image field is used as a path to a .wasm or .wat file

        #[cfg(all(target_os = "linux", feature = "hyperlight"))]
        {
            let image_path = Path::new(&config.image);

            // If image looks like a file path, try to load it
            if image_path.exists() {
                let wasm_bytes = load_wasm_file(image_path)?;
                self.init_with_wasm(&wasm_bytes)?;
            } else {
                // Just mark as ready - wasm can be loaded later via init_with_wasm
                self.running = true;
            }
            Ok(())
        }

        #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
        {
            let _ = config;
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
