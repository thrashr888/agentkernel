//! Input validation for security-critical operations.
//!
//! All user-provided inputs that could be used in shell commands, file paths,
//! or security-sensitive contexts MUST be validated through this module.

use anyhow::{Result, bail};
use std::path::Path;

/// Maximum length for sandbox names
const MAX_SANDBOX_NAME_LEN: usize = 63;

/// Maximum length for runtime names
const MAX_RUNTIME_NAME_LEN: usize = 32;

/// Allowed runtimes (validated against this list to prevent path traversal)
const ALLOWED_RUNTIMES: &[&str] = &[
    "base", "python", "node", "go", "rust", "ruby", "java", "c", "dotnet",
];

/// Validate a sandbox name.
///
/// Valid sandbox names:
/// - Start with a letter or number
/// - Contain only alphanumeric characters, hyphens, and underscores
/// - Are between 1 and 63 characters long
/// - Do not start or end with a hyphen or underscore
///
/// # Security
/// This prevents command injection via sandbox names that are interpolated
/// into shell commands and file paths.
pub fn validate_sandbox_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Sandbox name cannot be empty");
    }

    if name.len() > MAX_SANDBOX_NAME_LEN {
        bail!(
            "Sandbox name too long (max {} characters)",
            MAX_SANDBOX_NAME_LEN
        );
    }

    // Must start with alphanumeric
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        bail!("Sandbox name must start with a letter or number");
    }

    // Must end with alphanumeric
    let last = name.chars().last().unwrap();
    if !last.is_ascii_alphanumeric() {
        bail!("Sandbox name must end with a letter or number");
    }

    // Check all characters
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' {
            bail!(
                "Sandbox name contains invalid character '{}'. Only letters, numbers, hyphens, and underscores are allowed",
                ch
            );
        }
    }

    // Check for consecutive hyphens/underscores (common in injection attempts)
    if name.contains("--") || name.contains("__") || name.contains("-_") || name.contains("_-") {
        bail!("Sandbox name cannot contain consecutive hyphens or underscores");
    }

    Ok(())
}

/// Validate a runtime name against the allowlist.
///
/// # Security
/// This prevents path traversal attacks where a malicious runtime name like
/// `../../../etc/passwd` could be used to read arbitrary files.
pub fn validate_runtime(runtime: &str) -> Result<()> {
    if runtime.is_empty() {
        bail!("Runtime name cannot be empty");
    }

    if runtime.len() > MAX_RUNTIME_NAME_LEN {
        bail!(
            "Runtime name too long (max {} characters)",
            MAX_RUNTIME_NAME_LEN
        );
    }

    // Check against allowlist
    if !ALLOWED_RUNTIMES.contains(&runtime) {
        bail!(
            "Unknown runtime '{}'. Allowed runtimes: {}",
            runtime,
            ALLOWED_RUNTIMES.join(", ")
        );
    }

    Ok(())
}

/// Validate a working directory path for Seatbelt profiles.
///
/// # Security
/// This prevents SBPL injection via malicious path strings that could
/// break out of the string context and inject additional rules.
pub fn validate_seatbelt_path(path: &str) -> Result<String> {
    if path.is_empty() {
        bail!("Path cannot be empty");
    }

    // Check for characters that could break SBPL syntax
    // SBPL uses Lisp-like syntax, so we need to escape quotes and parens
    let dangerous_chars = ['"', ')', '(', '\n', '\r', '\0'];
    for ch in dangerous_chars {
        if path.contains(ch) {
            bail!(
                "Path contains invalid character for Seatbelt profile: {:?}",
                ch
            );
        }
    }

    // Ensure it's an absolute path
    if !path.starts_with('/') {
        bail!("Seatbelt working directory must be an absolute path");
    }

    // Normalize the path to prevent traversal
    let normalized = Path::new(path);
    if normalized
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Path cannot contain parent directory references (..)");
    }

    Ok(path.to_string())
}

/// Sanitize a Docker image name.
///
/// # Security
/// Validates that the image name follows Docker's naming conventions
/// to prevent injection via malformed image references.
pub fn validate_docker_image(image: &str) -> Result<()> {
    if image.is_empty() {
        bail!("Docker image name cannot be empty");
    }

    if image.len() > 256 {
        bail!("Docker image name too long (max 256 characters)");
    }

    // Docker image names can contain:
    // - Lowercase letters, digits
    // - Separators: period, hyphen, underscore
    // - Slashes for namespacing
    // - Colons for tags
    // - @ for digests
    for ch in image.chars() {
        if !ch.is_ascii_alphanumeric()
            && ch != '.'
            && ch != '-'
            && ch != '_'
            && ch != '/'
            && ch != ':'
            && ch != '@'
        {
            bail!(
                "Docker image name contains invalid character '{}'. Use only alphanumeric characters, periods, hyphens, underscores, slashes, colons, and @",
                ch
            );
        }
    }

    // Check for obvious shell injection attempts
    let dangerous_patterns = ["$(", "`", "&&", "||", ";", "|", ">", "<", "\n"];
    for pattern in dangerous_patterns {
        if image.contains(pattern) {
            bail!("Docker image name contains suspicious pattern: {}", pattern);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_sandbox_names() {
        assert!(validate_sandbox_name("my-sandbox").is_ok());
        assert!(validate_sandbox_name("test123").is_ok());
        assert!(validate_sandbox_name("My_Sandbox_1").is_ok());
        assert!(validate_sandbox_name("a").is_ok());
        assert!(validate_sandbox_name("a1b2c3").is_ok());
    }

    #[test]
    fn test_invalid_sandbox_names() {
        // Empty
        assert!(validate_sandbox_name("").is_err());

        // Too long
        assert!(validate_sandbox_name(&"a".repeat(64)).is_err());

        // Invalid characters
        assert!(validate_sandbox_name("test;rm -rf /").is_err());
        assert!(validate_sandbox_name("test$(whoami)").is_err());
        assert!(validate_sandbox_name("test`id`").is_err());
        assert!(validate_sandbox_name("test&& echo hi").is_err());
        assert!(validate_sandbox_name("../etc/passwd").is_err());

        // Invalid start/end
        assert!(validate_sandbox_name("-test").is_err());
        assert!(validate_sandbox_name("test-").is_err());
        assert!(validate_sandbox_name("_test").is_err());
        assert!(validate_sandbox_name("test_").is_err());

        // Consecutive separators
        assert!(validate_sandbox_name("test--name").is_err());
        assert!(validate_sandbox_name("test__name").is_err());
    }

    #[test]
    fn test_valid_runtimes() {
        assert!(validate_runtime("base").is_ok());
        assert!(validate_runtime("python").is_ok());
        assert!(validate_runtime("node").is_ok());
        assert!(validate_runtime("rust").is_ok());
    }

    #[test]
    fn test_invalid_runtimes() {
        // Not in allowlist
        assert!(validate_runtime("unknown").is_err());
        assert!(validate_runtime("../../../etc/passwd").is_err());
        assert!(validate_runtime("base; rm -rf /").is_err());
    }

    #[test]
    fn test_valid_seatbelt_paths() {
        assert!(validate_seatbelt_path("/tmp/test").is_ok());
        assert!(validate_seatbelt_path("/Users/test/workspace").is_ok());
        assert!(validate_seatbelt_path("/var/folders/abc").is_ok());
    }

    #[test]
    fn test_invalid_seatbelt_paths() {
        // Empty
        assert!(validate_seatbelt_path("").is_err());

        // Relative path
        assert!(validate_seatbelt_path("tmp/test").is_err());

        // Path traversal
        assert!(validate_seatbelt_path("/tmp/../etc/passwd").is_err());

        // SBPL injection attempts
        assert!(validate_seatbelt_path("/tmp\")(allow default)\"").is_err());
        assert!(validate_seatbelt_path("/tmp\")").is_err());
    }

    #[test]
    fn test_valid_docker_images() {
        assert!(validate_docker_image("alpine:3.20").is_ok());
        assert!(validate_docker_image("python:3.12-alpine").is_ok());
        assert!(validate_docker_image("ghcr.io/user/image:latest").is_ok());
        assert!(validate_docker_image("image@sha256:abc123").is_ok());
    }

    #[test]
    fn test_invalid_docker_images() {
        // Empty
        assert!(validate_docker_image("").is_err());

        // Injection attempts
        assert!(validate_docker_image("alpine; rm -rf /").is_err());
        assert!(validate_docker_image("alpine$(whoami)").is_err());
        assert!(validate_docker_image("alpine`id`").is_err());
    }
}
