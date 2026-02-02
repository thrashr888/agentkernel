//! Git context utilities for per-branch sandbox naming.
//!
//! Detects the current git project name and branch to generate
//! deterministic sandbox names like `myproject-feature-auth`.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Git context for the current working directory.
#[derive(Debug, Clone)]
pub struct GitContext {
    /// Project name (directory name of the repo root)
    pub project: String,
    /// Current branch name, sanitized for use as a sandbox name
    pub branch: String,
}

impl GitContext {
    /// Generate a sandbox name from the git context: `{project}-{branch}`
    pub fn sandbox_name(&self) -> String {
        format!("{}-{}", self.project, self.branch)
    }
}

/// Detect git context from the current working directory.
pub fn detect() -> Result<GitContext> {
    detect_in(".")
}

/// Detect git context from a specific directory.
pub fn detect_in(dir: &str) -> Result<GitContext> {
    let project = git_project_name(dir)?;
    let branch = git_branch_name(dir)?;
    Ok(GitContext { project, branch })
}

/// Get the project name from the git repo root directory name.
fn git_project_name(dir: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Not a git repository");
    }

    let toplevel = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let name = Path::new(&toplevel)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(sanitize_name(&name))
}

/// Get the current branch name, sanitized for sandbox naming.
fn git_branch_name(dir: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get current branch");
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch == "HEAD" {
        // Detached HEAD — use short SHA instead
        let sha_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(dir)
            .output()
            .context("Failed to get HEAD SHA")?;
        let sha = String::from_utf8_lossy(&sha_output.stdout)
            .trim()
            .to_string();
        return Ok(sanitize_name(&sha));
    }

    Ok(sanitize_name(&branch))
}

/// Sanitize a string for use as a sandbox name component.
/// Replaces `/`, spaces, and other special chars with `-`, lowercases, and trims.
fn sanitize_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['/', '\\', ' ', '_', '.'], "-")
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("feature/auth"), "feature-auth");
        assert_eq!(sanitize_name("my_project"), "my-project");
        assert_eq!(sanitize_name("UPPER.Case"), "upper-case");
        assert_eq!(sanitize_name("--trimmed--"), "trimmed");
        assert_eq!(sanitize_name("fix/bug 42"), "fix-bug-42");
    }

    #[test]
    fn test_detect_in_current_repo() {
        // This test only works when run inside a git repo
        if let Ok(ctx) = detect() {
            assert!(!ctx.project.is_empty());
            assert!(!ctx.branch.is_empty());
            let name = ctx.sandbox_name();
            assert!(name.contains('-') || !ctx.branch.is_empty());
        }
    }

    #[test]
    fn test_sandbox_name_format() {
        let ctx = GitContext {
            project: "myproject".to_string(),
            branch: "feature-auth".to_string(),
        };
        assert_eq!(ctx.sandbox_name(), "myproject-feature-auth");
    }

    #[test]
    fn test_detect_in_non_git_dir() {
        let result = detect_in("/tmp");
        // /tmp is typically not a git repo
        assert!(result.is_err());
    }
}
