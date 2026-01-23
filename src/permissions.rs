//! Permission profiles for sandbox security.
//!
//! Defines security profiles that control what sandboxed code can access.

use serde::{Deserialize, Serialize};

/// Security profile levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityProfile {
    /// Full access - network, all mounts, env passthrough
    Permissive,
    /// Balanced - network allowed, limited mounts, filtered env
    #[default]
    Moderate,
    /// Maximum isolation - no network, no mounts, clean env
    Restrictive,
    /// Custom profile defined by explicit permissions
    Custom,
}

impl SecurityProfile {
    /// Get the default permissions for this profile
    pub fn permissions(&self) -> Permissions {
        match self {
            SecurityProfile::Permissive => Permissions {
                network: true,
                mount_cwd: true,
                mount_home: true,
                pass_env: true,
                allow_privileged: false,
                read_only_root: false,
                max_memory_mb: None,
                max_cpu_percent: None,
            },
            SecurityProfile::Moderate => Permissions {
                network: true,
                mount_cwd: false,
                mount_home: false,
                pass_env: false,
                allow_privileged: false,
                read_only_root: false,
                max_memory_mb: Some(512),
                max_cpu_percent: Some(100),
            },
            SecurityProfile::Restrictive => Permissions {
                network: false,
                mount_cwd: false,
                mount_home: false,
                pass_env: false,
                allow_privileged: false,
                read_only_root: true,
                max_memory_mb: Some(256),
                max_cpu_percent: Some(50),
            },
            SecurityProfile::Custom => Permissions::default(),
        }
    }

    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "permissive" => Some(SecurityProfile::Permissive),
            "moderate" => Some(SecurityProfile::Moderate),
            "restrictive" => Some(SecurityProfile::Restrictive),
            "custom" => Some(SecurityProfile::Custom),
            _ => None,
        }
    }
}

/// Detailed permissions for sandbox execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    /// Allow network access
    pub network: bool,
    /// Mount current working directory
    pub mount_cwd: bool,
    /// Mount home directory (read-only)
    pub mount_home: bool,
    /// Pass through host environment variables
    pub pass_env: bool,
    /// Allow privileged operations (dangerous)
    pub allow_privileged: bool,
    /// Make root filesystem read-only
    pub read_only_root: bool,
    /// Maximum memory in MB (None = unlimited)
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU percentage (None = unlimited)
    pub max_cpu_percent: Option<u32>,
}

impl Default for Permissions {
    fn default() -> Self {
        SecurityProfile::Moderate.permissions()
    }
}

impl Permissions {
    /// Convert permissions to Docker run arguments
    pub fn to_docker_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Network
        if !self.network {
            args.push("--network=none".to_string());
        }

        // Memory limit
        if let Some(mem) = self.max_memory_mb {
            args.push(format!("--memory={}m", mem));
            // Disable OOM killer overhead - container will be constrained but not killed
            args.push("--oom-kill-disable".to_string());
        }

        // CPU limit
        if let Some(cpu) = self.max_cpu_percent {
            // Docker uses CPU period/quota, simplified here
            args.push(format!("--cpus={:.2}", cpu as f64 / 100.0));
        }

        // Read-only root
        if self.read_only_root {
            args.push("--read-only".to_string());
            // Add tmpfs for /tmp so programs can still write temp files
            args.push("--tmpfs=/tmp:rw,noexec,nosuid,size=64m".to_string());
        }

        // Security options (always apply some baseline security)
        if !self.allow_privileged {
            args.push("--security-opt=no-new-privileges".to_string());
            args.push("--cap-drop=ALL".to_string());
            // Add back minimal caps needed for most programs
            args.push("--cap-add=CHOWN".to_string());
            args.push("--cap-add=SETUID".to_string());
            args.push("--cap-add=SETGID".to_string());
        }

        args
    }

    /// Get environment variables to pass (or block)
    pub fn get_env_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if self.pass_env {
            // Pass through common useful env vars
            for var in ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"] {
                if let Ok(val) = std::env::var(var) {
                    args.push("-e".to_string());
                    args.push(format!("{}={}", var, val));
                }
            }
        }

        args
    }

    /// Get mount arguments
    pub fn get_mount_args(&self, cwd: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();

        if self.mount_cwd
            && let Some(dir) = cwd
        {
            args.push("-v".to_string());
            args.push(format!("{}:/workspace:rw", dir));
            args.push("-w".to_string());
            args.push("/workspace".to_string());
        }

        if self.mount_home
            && let Ok(home) = std::env::var("HOME")
        {
            args.push("-v".to_string());
            args.push(format!("{}:/home/user:ro", home));
        }

        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_profiles() {
        let permissive = SecurityProfile::Permissive.permissions();
        assert!(permissive.network);
        assert!(permissive.mount_cwd);

        let moderate = SecurityProfile::Moderate.permissions();
        assert!(moderate.network);
        assert!(!moderate.mount_cwd);

        let restrictive = SecurityProfile::Restrictive.permissions();
        assert!(!restrictive.network);
        assert!(!restrictive.mount_cwd);
        assert!(restrictive.read_only_root);
    }

    #[test]
    fn test_docker_args() {
        let restrictive = SecurityProfile::Restrictive.permissions();
        let args = restrictive.to_docker_args();

        assert!(args.contains(&"--network=none".to_string()));
        assert!(args.contains(&"--read-only".to_string()));
    }
}
