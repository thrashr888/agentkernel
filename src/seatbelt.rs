//! macOS Seatbelt Sandbox Backend
//!
//! Lightweight sandboxing for macOS using sandbox-exec (Seatbelt).
//! Provides process isolation without Docker or virtualization overhead.
//!
//! Note: This only works on macOS and requires the `sandbox-exec` command.

use anyhow::{Result, bail};
#[cfg(target_os = "macos")]
use anyhow::Context;
use std::process::Output;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

/// Security profile for Seatbelt sandbox
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SeatbeltProfile {
    /// Minimal restrictions - network, most filesystem access allowed
    Permissive,
    /// Moderate restrictions - network allowed, limited filesystem
    #[default]
    Moderate,
    /// Maximum restrictions - no network, read-only filesystem
    Restrictive,
}

/// Seatbelt sandbox for running commands on macOS
#[allow(dead_code)]
pub struct SeatbeltSandbox {
    profile: SeatbeltProfile,
    working_dir: Option<String>,
}

impl SeatbeltSandbox {
    /// Create a new Seatbelt sandbox with the given profile
    #[allow(dead_code)]
    pub fn new(profile: SeatbeltProfile) -> Self {
        Self {
            profile,
            working_dir: None,
        }
    }

    /// Set the working directory for commands
    #[allow(dead_code)]
    pub fn with_working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Check if Seatbelt is available on this system
    #[allow(dead_code)]
    pub fn is_available() -> bool {
        #[cfg(target_os = "macos")]
        {
            Command::new("sandbox-exec")
                .arg("-h")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Generate the Seatbelt profile SBPL (Sandbox Profile Language)
    #[allow(dead_code)]
    fn generate_profile(&self) -> String {
        match self.profile {
            SeatbeltProfile::Permissive => {
                // Allow most operations
                r#"(version 1)
(allow default)
(deny file-write* (subpath "/System"))
(deny file-write* (subpath "/Library"))
(deny file-write* (subpath "/usr"))
(deny process-exec* (subpath "/System"))
"#
                .to_string()
            }
            SeatbeltProfile::Moderate => {
                // Allow network, limited filesystem
                let working_dir = self
                    .working_dir
                    .as_deref()
                    .unwrap_or("/tmp/agentkernel-sandbox");
                format!(
                    r#"(version 1)
(deny default)
(allow signal (target self))
(allow process-fork)
(allow process-exec)
(allow sysctl-read)
(allow mach-lookup)
(allow mach-register)
(allow ipc-posix*)
(allow system-socket)

; Network access
(allow network*)

; Allow read access to system paths
(allow file-read* (subpath "/"))

; Allow write to working directory
(allow file-write* (subpath "{}"))
(allow file-write* (subpath "/tmp"))
(allow file-write* (subpath "/var/folders"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath "/private/var/folders"))

; Allow executing binaries
(allow process-exec (subpath "/usr/bin"))
(allow process-exec (subpath "/usr/local/bin"))
(allow process-exec (subpath "/opt/homebrew/bin"))
(allow process-exec (subpath "/bin"))
(allow process-exec (subpath "/sbin"))
"#,
                    working_dir
                )
            }
            SeatbeltProfile::Restrictive => {
                // No network, minimal filesystem
                let working_dir = self
                    .working_dir
                    .as_deref()
                    .unwrap_or("/tmp/agentkernel-sandbox");
                format!(
                    r#"(version 1)
(deny default)
(allow signal (target self))
(allow process-fork)
(allow process-exec)
(allow sysctl-read)
(allow mach-lookup)
(allow ipc-posix*)

; NO network access

; Allow read access to essential paths
(allow file-read* (subpath "/usr"))
(allow file-read* (subpath "/bin"))
(allow file-read* (subpath "/sbin"))
(allow file-read* (subpath "/opt"))
(allow file-read* (subpath "/Library/Frameworks"))
(allow file-read* (subpath "/System/Library"))
(allow file-read* (subpath "/private/etc"))
(allow file-read* (subpath "/dev"))

; Allow read/write to working directory only
(allow file-read* (subpath "{}"))
(allow file-write* (subpath "{}"))
(allow file-write* (subpath "/tmp"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath "/dev/null"))
(allow file-write* (subpath "/dev/tty"))

; Allow executing binaries
(allow process-exec (subpath "/usr/bin"))
(allow process-exec (subpath "/bin"))
(allow process-exec (subpath "/opt/homebrew/bin"))
"#,
                    working_dir, working_dir
                )
            }
        }
    }

    /// Run a command in the sandbox
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    pub fn run(&self, command: &[String]) -> Result<Output> {
        if command.is_empty() {
            bail!("Empty command");
        }

        let profile = self.generate_profile();

        // Create a temporary file for the profile
        let profile_path =
            std::env::temp_dir().join(format!("agentkernel-seatbelt-{}.sb", std::process::id()));
        std::fs::write(&profile_path, &profile).context("Failed to write Seatbelt profile")?;

        // Build the sandboxed command
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f").arg(&profile_path);
        cmd.arg(command[0].clone());
        cmd.args(&command[1..]);

        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        let output = cmd.output().context("Failed to run sandboxed command")?;

        // Clean up profile file
        let _ = std::fs::remove_file(&profile_path);

        Ok(output)
    }

    /// Stub for non-macOS platforms
    #[cfg(not(target_os = "macos"))]
    #[allow(dead_code)]
    pub fn run(&self, _command: &[String]) -> Result<Output> {
        bail!("Seatbelt sandbox is only available on macOS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_generation() {
        let sandbox =
            SeatbeltSandbox::new(SeatbeltProfile::Restrictive).with_working_dir("/tmp/test");

        let profile = sandbox.generate_profile();
        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("/tmp/test"));
    }

    #[test]
    fn test_permissive_profile() {
        let sandbox = SeatbeltSandbox::new(SeatbeltProfile::Permissive);
        let profile = sandbox.generate_profile();
        assert!(profile.contains("(allow default)"));
    }
}
