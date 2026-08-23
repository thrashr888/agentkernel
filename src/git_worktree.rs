//! Safe, host-side Git worktree management for agent sandboxes.
//!
//! A managed worktree is deliberately kept outside the user's repository and
//! is removed only through Git's worktree plumbing.  Branches are retained on
//! cleanup so commits made by an agent cannot be discarded accidentally.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::validation;

/// Prefix used for branches created for sandbox worktrees.
pub const BRANCH_PREFIX: &str = "agentkernel/sandbox/";

/// Persisted metadata describing a managed checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWorktree {
    pub repository: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub base_ref: String,
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", repo.display()))?;
    if !output.status.success() {
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn canonical_git_root(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("Git workspace '{}' is not accessible", path.display()))?;
    if !path.is_dir() {
        bail!("Git workspace '{}' is not a directory", path.display());
    }
    let root = PathBuf::from(git(&path, &["rev-parse", "--show-toplevel"])?);
    let root = root
        .canonicalize()
        .with_context(|| format!("Git repository '{}' is not accessible", root.display()))?;
    if !root.is_dir() {
        bail!("Git repository '{}' is not a directory", root.display());
    }
    Ok(root)
}

fn canonical_git_common_dir(path: &Path) -> Result<PathBuf> {
    let common_dir = PathBuf::from(git(path, &["rev-parse", "--git-common-dir"])?);
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        path.join(common_dir)
    };
    common_dir.canonicalize().with_context(|| {
        format!(
            "Git common directory '{}' is not accessible",
            common_dir.display()
        )
    })
}

fn ensure_within(path: &Path, root: &Path) -> Result<()> {
    if path == root || !path.starts_with(root) {
        bail!(
            "managed Git worktree path '{}' escapes '{}', refusing to continue",
            path.display(),
            root.display()
        );
    }
    Ok(())
}

/// Verify that two paths belong to the same Git repository, including when
/// either path is itself a linked worktree.
pub fn same_repository(first: &Path, second: &Path) -> Result<bool> {
    Ok(canonical_git_common_dir(&canonical_git_root(first)?)?
        == canonical_git_common_dir(&canonical_git_root(second)?)?)
}

fn verify_identity(worktree: &ManagedWorktree, managed_root: &Path) -> Result<PathBuf> {
    if !worktree.branch.starts_with(BRANCH_PREFIX) {
        bail!(
            "refusing to use non-AgentKernel Git branch '{}' as a managed worktree",
            worktree.branch
        );
    }
    let managed_root = managed_root.canonicalize().with_context(|| {
        format!(
            "managed Git worktree root '{}' is not accessible",
            managed_root.display()
        )
    })?;
    if !worktree.path.exists() {
        bail!(
            "managed Git worktree '{}' does not exist",
            worktree.path.display()
        );
    }
    let path = worktree.path.canonicalize().with_context(|| {
        format!(
            "managed Git worktree '{}' is not accessible",
            worktree.path.display()
        )
    })?;
    ensure_within(&path, &managed_root)?;

    let repository = canonical_git_root(&worktree.repository)?;
    let recorded_root = canonical_git_root(&path)?;
    let repository_common_dir = canonical_git_common_dir(&repository)?;
    let worktree_common_dir = canonical_git_common_dir(&recorded_root)?;
    if repository_common_dir != worktree_common_dir {
        bail!(
            "Git worktree '{}' belongs to a different Git repository ('{}'), not recorded repository '{}'; refusing cleanup",
            path.display(),
            worktree_common_dir.display(),
            repository.display()
        );
    }
    let actual_branch = git(&path, &["branch", "--show-current"])?;
    if actual_branch != worktree.branch {
        bail!(
            "Git worktree '{}' is on '{}', not recorded branch '{}'; refusing cleanup",
            path.display(),
            actual_branch,
            worktree.branch
        );
    }
    Ok(path)
}

/// Verify the recorded checkout identity without changing the filesystem.
pub fn verify(worktree: &ManagedWorktree, managed_root: &Path) -> Result<()> {
    let _ = verify_identity(worktree, managed_root)?;
    Ok(())
}

/// Create a dedicated branch and checkout under `managed_root`.
///
/// `sandbox_id` should be a stable, opaque sandbox UUID.  The function never
/// invokes a shell and refuses to reuse an existing path, which makes a
/// partially-created checkout fail closed instead of overwriting user data.
pub fn create(
    repository: &Path,
    managed_root: &Path,
    sandbox_name: &str,
    sandbox_id: &str,
) -> Result<ManagedWorktree> {
    validation::validate_sandbox_name(sandbox_name)?;
    validation::validate_git_ref(sandbox_id)?;

    let repository = canonical_git_root(repository)?;
    std::fs::create_dir_all(managed_root)
        .with_context(|| format!("failed to create {}", managed_root.display()))?;
    let managed_root = managed_root.canonicalize().with_context(|| {
        format!(
            "managed Git worktree root '{}' is not accessible",
            managed_root.display()
        )
    })?;

    // UUIDs are validated above, but keep the final path component explicit so
    // this remains safe if the caller ever changes its ID format.
    let path = managed_root.join(sandbox_id);
    ensure_within(&path, &managed_root)?;
    // `exists()` follows symlinks and would miss a dangling link. Refuse any
    // existing directory entry so a pre-planted link cannot redirect Git's
    // checkout outside the managed root.
    if std::fs::symlink_metadata(&path).is_ok() {
        bail!(
            "managed Git worktree path '{}' already exists; refusing to overwrite it",
            path.display()
        );
    }

    let branch = format!("{}{}-{}", BRANCH_PREFIX, sandbox_name, sandbox_id);
    validation::validate_git_ref(&branch)?;
    let base_ref = git(&repository, &["rev-parse", "HEAD"])?;
    if base_ref.is_empty() {
        bail!(
            "Git repository '{}' has no HEAD commit",
            repository.display()
        );
    }

    let path_string = path.to_string_lossy().into_owned();
    let add_result = Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["worktree", "add", "-b", &branch])
        .arg(&path_string)
        .arg(&base_ref)
        .output()
        .context("failed to create Git worktree")?;
    if !add_result.status.success() {
        bail!(
            "git worktree add failed for '{}': {}",
            path.display(),
            String::from_utf8_lossy(&add_result.stderr).trim()
        );
    }

    Ok(ManagedWorktree {
        repository,
        path,
        branch,
        base_ref,
    })
}

fn ensure_clean(worktree: &ManagedWorktree, managed_root: &Path) -> Result<PathBuf> {
    if !worktree.branch.starts_with(BRANCH_PREFIX) {
        bail!(
            "refusing to remove non-AgentKernel Git branch '{}'",
            worktree.branch
        );
    }
    let managed_root = managed_root.canonicalize().with_context(|| {
        format!(
            "managed Git worktree root '{}' is not accessible",
            managed_root.display()
        )
    })?;
    if !worktree.path.exists() {
        // A missing checkout is already clean, but still validate the
        // recorded path before accepting it as managed state.
        ensure_within(&worktree.path, &managed_root)?;
        return Ok(worktree.path.clone());
    }
    let path = verify_identity(worktree, &managed_root)?;

    // Never discard agent output implicitly. This also catches untracked
    // files, which are part of the agent's work even when Git has not staged
    // them yet.
    let status = git(
        &path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    if !status.is_empty() {
        bail!(
            "Git worktree '{}' has uncommitted changes; commit or clean it before removing sandbox (branch '{}' is preserved)",
            path.display(),
            worktree.branch
        );
    }
    Ok(path)
}

/// Verify that a managed checkout can be removed without discarding data.
///
/// This is intentionally non-mutating and should be called before tearing
/// down a sandbox backend. It checks ownership, branch identity, and tracked,
/// untracked, and ignored changes.
pub fn ensure_clean_removable(worktree: &ManagedWorktree, managed_root: &Path) -> Result<()> {
    let _ = ensure_clean(worktree, managed_root)?;
    Ok(())
}

/// Remove a managed checkout while preserving its branch and any commits.
///
/// The caller must pass the expected managed root. Both the repository and
/// checkout are verified before Git is asked to remove anything.
pub fn remove(worktree: &ManagedWorktree, managed_root: &Path) -> Result<()> {
    let path = ensure_clean(worktree, managed_root)?;
    if !worktree.path.exists() {
        return Ok(());
    }
    let repository = canonical_git_root(&worktree.repository)?;

    let path_string = path.to_string_lossy().into_owned();
    let output = Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["worktree", "remove"])
        .arg(&path_string)
        .output()
        .context("failed to remove Git worktree")?;
    if !output.status.success() {
        bail!(
            "git worktree remove failed for '{}': {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.name", "Test Agent"]);
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        fs::write(dir.path().join("README.md"), "base\n").unwrap();
        git(dir.path(), &["add", "README.md"]);
        git(dir.path(), &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn creates_and_removes_only_managed_checkout() {
        let repository = repo();
        let root = tempfile::tempdir().unwrap();
        let worktree = create(repository.path(), root.path(), "demo", "sandbox-id").unwrap();
        assert!(worktree.path.join("README.md").exists());
        assert!(worktree.branch.starts_with(BRANCH_PREFIX));

        fs::write(worktree.path.join("agent.txt"), "keep branch\n").unwrap();
        let error = ensure_clean_removable(&worktree, root.path()).unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"));
        assert!(worktree.path.join("agent.txt").exists());
        let error = remove(&worktree, root.path()).unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"));
        assert!(worktree.path.join("agent.txt").exists());
        fs::remove_file(worktree.path.join("agent.txt")).unwrap();
        remove(&worktree, root.path()).unwrap();
        assert!(!worktree.path.exists());
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repository.path())
                .args([
                    "show-ref",
                    "--verify",
                    &format!("refs/heads/{}", worktree.branch),
                ])
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    #[test]
    fn refuses_path_outside_managed_root() {
        let repository = repo();
        let root = tempfile::tempdir().unwrap();
        let worktree = ManagedWorktree {
            repository: repository.path().to_path_buf(),
            path: repository.path().join("outside"),
            branch: format!("{}demo-id", BRANCH_PREFIX),
            base_ref: "HEAD".to_string(),
        };
        let error = remove(&worktree, root.path()).unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn rejects_existing_destination() {
        let repository = repo();
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("sandbox-id")).unwrap();
        let error = create(repository.path(), root.path(), "demo", "sandbox-id").unwrap_err();
        assert!(error.to_string().contains("already exists"));
    }
}
