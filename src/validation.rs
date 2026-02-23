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

/// Maximum length for Git source URLs
const MAX_GIT_URL_LEN: usize = 2048;

/// Maximum length for Git refs
const MAX_GIT_REF_LEN: usize = 255;

/// Maximum length for exec working directory paths
const MAX_WORKDIR_LEN: usize = 1024;

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

/// Validate a Git source URL accepted by sandbox create flows.
///
/// Allowed formats:
/// - `https://host/org/repo.git`
/// - `ssh://host/org/repo.git`
/// - `git@host:org/repo.git`
///
/// # Security
/// Rejects local file paths, unsupported protocols, and suspicious shell tokens.
pub fn validate_git_source_url(url: &str) -> Result<()> {
    if url.is_empty() {
        bail!("Git source URL cannot be empty");
    }
    if url.len() > MAX_GIT_URL_LEN {
        bail!(
            "Git source URL too long (max {} characters)",
            MAX_GIT_URL_LEN
        );
    }

    if url
        .chars()
        .any(|ch| ch.is_ascii_control() || ch.is_whitespace())
    {
        bail!("Git source URL cannot contain whitespace or control characters");
    }

    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") {
        bail!("Git source URL must use HTTPS or SSH");
    }
    if lower.starts_with("file://")
        || lower.starts_with('/')
        || lower.starts_with("./")
        || lower.starts_with("../")
    {
        bail!("Local filesystem paths are not allowed for Git source URLs");
    }

    let dangerous_patterns = ["$(", "`", "&&", "||", ";", "|", ">", "<"];
    for pattern in dangerous_patterns {
        if url.contains(pattern) {
            bail!("Git source URL contains suspicious pattern: {}", pattern);
        }
    }

    let is_https = lower.starts_with("https://");
    let is_ssh = lower.starts_with("ssh://");
    let is_scp_style = url.starts_with("git@");
    if !(is_https || is_ssh || is_scp_style) {
        bail!("Git source URL must use https://, ssh://, or git@host:path format");
    }

    if is_https || is_ssh {
        let Some((_, rest)) = url.split_once("://") else {
            bail!("Invalid Git source URL");
        };
        let host_port = rest.split('/').next().unwrap_or("");
        let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
        let host = host_port.split(':').next().unwrap_or(host_port);
        if host.is_empty()
            || !host
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        {
            bail!("Git source URL must include a valid host");
        }
    }

    if is_scp_style {
        let rest = &url["git@".len()..];
        let Some((host, path)) = rest.split_once(':') else {
            bail!("SCP-style Git URL must be in git@host:path format");
        };
        if host.is_empty() || path.is_empty() {
            bail!("SCP-style Git URL must include host and repository path");
        }
        if !host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        {
            bail!("SCP-style Git URL contains invalid host");
        }
    }

    Ok(())
}

/// Validate a Git ref (branch, tag, or commit-ish).
///
/// # Security
/// Prevents option-like refs and malformed names that can trigger unexpected
/// `git checkout` behavior.
pub fn validate_git_ref(git_ref: &str) -> Result<()> {
    if git_ref.is_empty() {
        bail!("Git ref cannot be empty");
    }
    if git_ref.len() > MAX_GIT_REF_LEN {
        bail!("Git ref too long (max {} characters)", MAX_GIT_REF_LEN);
    }
    if git_ref.starts_with('-') {
        bail!("Git ref cannot start with '-'");
    }
    if git_ref.starts_with('/') || git_ref.ends_with('/') {
        bail!("Git ref cannot start or end with '/'");
    }
    if git_ref.starts_with('.') || git_ref.ends_with('.') {
        bail!("Git ref cannot start or end with '.'");
    }
    if git_ref.contains("..") || git_ref.contains("//") || git_ref.contains("@{") {
        bail!("Git ref contains invalid sequence");
    }
    if git_ref
        .chars()
        .any(|ch| ch.is_ascii_control() || ch.is_whitespace())
    {
        bail!("Git ref cannot contain whitespace or control characters");
    }
    let disallowed = ['~', '^', ':', '?', '*', '[', '\\'];
    if git_ref.chars().any(|ch| disallowed.contains(&ch)) {
        bail!("Git ref contains invalid characters");
    }
    if !git_ref
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '/' || ch == '-' || ch == '_' || ch == '.')
    {
        bail!("Git ref contains unsupported characters");
    }

    Ok(())
}

/// Validate a sandbox exec working directory path.
///
/// # Security
/// Prevents path traversal and malformed values from being passed to sandbox runtimes.
pub fn validate_exec_workdir(workdir: &str) -> Result<()> {
    if workdir.is_empty() {
        bail!("Working directory cannot be empty");
    }
    if workdir.len() > MAX_WORKDIR_LEN {
        bail!(
            "Working directory too long (max {} characters)",
            MAX_WORKDIR_LEN
        );
    }
    if !workdir.starts_with('/') {
        bail!("Working directory must be an absolute path");
    }
    if workdir
        .chars()
        .any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
    {
        bail!("Working directory contains invalid control characters");
    }

    let normalized = Path::new(workdir);
    if normalized
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Working directory cannot contain parent directory references (..)");
    }

    Ok(())
}

/// Maximum length for label keys (following Kubernetes conventions)
const MAX_LABEL_KEY_LEN: usize = 63;

/// Maximum length for label values
const MAX_LABEL_VALUE_LEN: usize = 255;

/// Validate a label key=value pair.
///
/// Follows Kubernetes-like conventions:
/// - Keys: 1–63 chars, alphanumeric plus `-`, `_`, `.`
/// - Values: 1–255 chars, alphanumeric plus `-`, `_`, `.`, `/`
///
/// # Security
/// Prevents excessively long or malformed labels that could cause
/// storage/injection issues.
pub fn validate_label(key: &str, value: &str) -> Result<()> {
    let key = key.trim();
    let value = value.trim();

    if key.is_empty() {
        bail!("Label key must not be empty");
    }
    if key.len() > MAX_LABEL_KEY_LEN {
        bail!(
            "Label key '{}' too long (max {} characters)",
            key,
            MAX_LABEL_KEY_LEN
        );
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!(
            "Label key '{}' contains invalid characters (allowed: alphanumeric, '-', '_', '.')",
            key
        );
    }

    if value.is_empty() {
        bail!("Label value must not be empty");
    }
    if value.len() > MAX_LABEL_VALUE_LEN {
        bail!(
            "Label value too long (max {} characters)",
            MAX_LABEL_VALUE_LEN
        );
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
    {
        bail!(
            "Label value '{}' contains invalid characters (allowed: alphanumeric, '-', '_', '.', '/')",
            value
        );
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

    #[test]
    fn test_valid_git_source_urls() {
        assert!(validate_git_source_url("https://github.com/org/repo.git").is_ok());
        assert!(validate_git_source_url("ssh://github.com/org/repo.git").is_ok());
        assert!(validate_git_source_url("git@github.com:org/repo.git").is_ok());
    }

    #[test]
    fn test_invalid_git_source_urls() {
        assert!(validate_git_source_url("").is_err());
        assert!(validate_git_source_url("http://github.com/org/repo.git").is_err());
        assert!(validate_git_source_url("file:///tmp/repo").is_err());
        assert!(validate_git_source_url("../repo").is_err());
        assert!(validate_git_source_url("https://github.com/org/repo.git;rm -rf /").is_err());
    }

    #[test]
    fn test_valid_git_refs() {
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("feature/add-api").is_ok());
        assert!(validate_git_ref("v1.2.3").is_ok());
        assert!(validate_git_ref("a1b2c3d4").is_ok());
    }

    #[test]
    fn test_invalid_git_refs() {
        assert!(validate_git_ref("").is_err());
        assert!(validate_git_ref("-main").is_err());
        assert!(validate_git_ref("../main").is_err());
        assert!(validate_git_ref("main..next").is_err());
        assert!(validate_git_ref("main@{1}").is_err());
        assert!(validate_git_ref("main name").is_err());
        assert!(validate_git_ref("main:next").is_err());
    }

    #[test]
    fn test_valid_labels() {
        assert!(validate_label("env", "prod").is_ok());
        assert!(validate_label("team", "ml-ops").is_ok());
        assert!(validate_label("eval_run", "pr-123").is_ok());
        assert!(validate_label("app.version", "v1.2.3").is_ok());
        assert!(validate_label("scenario", "drift/s3").is_ok());
    }

    #[test]
    fn test_invalid_labels() {
        // Empty key
        assert!(validate_label("", "value").is_err());
        assert!(validate_label("  ", "value").is_err());

        // Empty value
        assert!(validate_label("key", "").is_err());
        assert!(validate_label("key", "  ").is_err());

        // Key too long
        assert!(validate_label(&"a".repeat(64), "value").is_err());

        // Value too long
        assert!(validate_label("key", &"a".repeat(256)).is_err());

        // Invalid characters in key
        assert!(validate_label("key=value", "v").is_err());
        assert!(validate_label("key:value", "v").is_err());
        assert!(validate_label("key value", "v").is_err());

        // Invalid characters in value
        assert!(validate_label("key", "val;ue").is_err());
        assert!(validate_label("key", "val$(cmd)").is_err());
    }

    #[test]
    fn test_valid_exec_workdir() {
        assert!(validate_exec_workdir("/workspace").is_ok());
        assert!(validate_exec_workdir("/workspace/project/src").is_ok());
    }

    #[test]
    fn test_invalid_exec_workdir() {
        assert!(validate_exec_workdir("").is_err());
        assert!(validate_exec_workdir("workspace").is_err());
        assert!(validate_exec_workdir("/workspace/../etc").is_err());
        assert!(validate_exec_workdir("/workspace\n/tmp").is_err());
    }
}
