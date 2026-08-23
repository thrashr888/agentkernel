//! Production VM-backed task executor.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::backend::ExecOptions;
use crate::permissions::SecurityProfile;
use crate::task_worker::TaskExecutor;
use crate::tasks::{TaskIsolation, TaskRecord};
use crate::validation;
use crate::vmm::VmManager;

/// Production executor backed by the existing VM manager.
///
/// A task's `sandbox` is the caller-selected template. Its `work_dir` must be
/// a Git checkout on the host; the worker creates a sibling worktree and mounts
/// that path into a fresh sandbox. This avoids copying credentials or mutable
/// state from the template sandbox.
#[derive(Clone)]
pub struct VmTaskExecutor {
    manager: Arc<RwLock<VmManager>>,
}

impl VmTaskExecutor {
    pub fn new(manager: Arc<RwLock<VmManager>>) -> Self {
        Self { manager }
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

    fn remove_checkout(path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(path).context("failed to remove task Git checkout")
    }

    fn create_checkout(repo: &Path, path: &Path, branch: &str, base_ref: &str) -> Result<()> {
        let path_string = path.to_string_lossy().into_owned();
        Self::git(
            repo,
            &["clone", "--no-local", "--no-checkout", ".", &path_string],
        )?;
        if let Err(error) = Self::git(path, &["checkout", "-b", branch, base_ref]) {
            let _ = Self::remove_checkout(path);
            return Err(error);
        }
        Ok(())
    }

    fn agent_command(agent: &str, prompt: &str) -> Result<Vec<String>> {
        let mut command = match agent {
            "claude" => vec!["claude", "--dangerously-skip-permissions", "-p"],
            "codex" => vec!["codex", "exec", "--full-auto"],
            "gemini" => vec!["gemini", "--yolo"],
            "opencode" => vec!["opencode", "run"],
            "amp" => vec!["amp", "--prompt"],
            "pi" => vec!["pi"],
            "copilot" => vec!["copilot", "-p"],
            _ => bail!("unsupported task agent '{agent}'"),
        }
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        command.push(prompt.to_string());
        Ok(command)
    }

    /// Capture the complete final tree against the task's starting commit.
    /// Staging is safe in the disposable worktree and, unlike plain
    /// `git diff`, includes files the agent created but did not add itself.
    fn review_diff(worktree: &Path, base_ref: &str) -> Result<String> {
        Self::git(worktree, &["add", "-A"])?;
        Self::git(worktree, &["diff", "--cached", "--binary", base_ref])
    }
}

#[async_trait]
impl TaskExecutor for VmTaskExecutor {
    async fn prepare(
        &mut self,
        task: &TaskRecord,
        planned: &TaskIsolation,
    ) -> Result<TaskIsolation> {
        let expected = TaskIsolation::planned(&task.id);
        if planned.sandbox != expected.sandbox || planned.branch != expected.branch {
            bail!("task isolation names do not match task ID");
        }
        validation::validate_git_ref(&planned.branch)?;

        let mut manager = self.manager.write().await;
        let template = manager
            .get_state(&task.sandbox)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("task template sandbox '{}' not found", task.sandbox))?;
        let base_dir = template
            .work_dir
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "task template sandbox '{}' has no host Git workspace",
                    task.sandbox
                )
            })?;
        let base_dir = base_dir.canonicalize().with_context(|| {
            format!("task workspace '{}' is not accessible", base_dir.display())
        })?;
        let repo = PathBuf::from(Self::git(&base_dir, &["rev-parse", "--show-toplevel"])?);
        let base_ref = Self::git(&repo, &["rev-parse", "HEAD"])?;

        let sandbox_name = &planned.sandbox;
        if manager.get_state(sandbox_name).is_some() {
            bail!("task sandbox '{sandbox_name}' already exists");
        }

        let worktree = manager.get_data_dir().join("task-worktrees").join(&task.id);
        std::fs::create_dir_all(worktree.parent().expect("worktree has a parent"))
            .context("failed to create task worktree directory")?;
        if worktree.exists() {
            Self::remove_checkout(&worktree)?;
        }
        let worktree_string = worktree.to_string_lossy().into_owned();
        Self::create_checkout(&repo, &worktree, &planned.branch, &base_ref)?;

        if let Err(error) = manager
            .create_with_agent(
                sandbox_name,
                &template.image,
                template.vcpus,
                template.memory_mb,
                None,
                Vec::new(),
                template.agent.clone(),
            )
            .await
        {
            let _ = Self::remove_checkout(&worktree);
            return Err(error)
                .with_context(|| format!("failed to create task sandbox '{sandbox_name}'"));
        }
        let mut labels: HashMap<String, String> = template.labels.clone();
        labels.insert("agentkernel.task_id".to_string(), task.id.clone());
        if let Err(error) = manager.set_labels(sandbox_name, &labels) {
            let _ = manager.remove(sandbox_name).await;
            let _ = Self::remove_checkout(&worktree);
            return Err(error).context("failed to mark task sandbox ownership");
        }
        if let Err(error) = manager.set_config_path(sandbox_name, template.config_path.clone()) {
            let _ = manager.remove(sandbox_name).await;
            let _ = Self::remove_checkout(&worktree);
            return Err(error).context("failed to propagate task Git identity config");
        }
        if let Some(init_script) = template.init_script.as_deref()
            && let Err(error) = manager.set_init_script(sandbox_name, init_script)
        {
            let _ = manager.remove(sandbox_name).await;
            let _ = Self::remove_checkout(&worktree);
            return Err(error).context("failed to propagate task template init script");
        }
        if let Err(error) = manager.set_work_dir(sandbox_name, Some(worktree_string.clone())) {
            let _ = manager.remove(sandbox_name).await;
            let _ = Self::remove_checkout(&worktree);
            return Err(error).context("failed to configure task workspace");
        }
        if !manager.is_running(sandbox_name) {
            let mut permissions = SecurityProfile::Moderate.permissions();
            permissions.mount_cwd = true;
            if let Err(error) = manager
                .start_with_permissions(sandbox_name, &permissions)
                .await
            {
                let _ = manager.remove(sandbox_name).await;
                let _ = Self::remove_checkout(&worktree);
                return Err(error)
                    .with_context(|| format!("failed to start task sandbox '{sandbox_name}'"));
            }
        }

        Ok(TaskIsolation {
            sandbox: planned.sandbox.clone(),
            branch: planned.branch.clone(),
            worktree: Some(task.id.clone()),
            base_ref: Some(base_ref),
        })
    }

    async fn execute(&mut self, task: &TaskRecord, isolation: &TaskIsolation) -> Result<String> {
        let mut manager = self.manager.write().await;
        let agent = manager
            .get_state(&isolation.sandbox)
            .and_then(|state| state.agent.clone())
            .ok_or_else(|| anyhow::anyhow!("task template has no configured agent CLI"))?;
        let command = Self::agent_command(&agent, &task.prompt)?;
        let output = manager
            .exec_cmd_full(
                &isolation.sandbox,
                &command,
                &ExecOptions {
                    workdir: Some("/workspace".to_string()),
                    ..Default::default()
                },
            )
            .await
            .context("agent execution failed")?;

        let worktree_id = isolation
            .worktree
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("task worktree was not persisted"))?;
        if worktree_id != task.id {
            bail!("task worktree identifier does not match task ID");
        }
        let worktree = manager
            .get_data_dir()
            .join("task-worktrees")
            .join(worktree_id);
        let base_ref = isolation
            .base_ref
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("task base ref was not persisted"))?;
        let diff = Self::review_diff(&worktree, base_ref)?;
        let diff = if diff.trim().is_empty() {
            format!(
                "Agent output:\n{}\n\nNo Git changes produced.",
                output.trim()
            )
        } else {
            diff
        };
        Ok(diff)
    }

    async fn cleanup(&mut self, task: &TaskRecord, isolation: &TaskIsolation) -> Result<()> {
        let mut cleanup_error = None;
        let expected = TaskIsolation::planned(&task.id);
        let expected_worktree = {
            let mut manager = self.manager.write().await;
            let expected_worktree = manager.get_data_dir().join("task-worktrees").join(&task.id);
            let owns_sandbox = manager
                .get_state(&isolation.sandbox)
                .and_then(|state| state.labels.get("agentkernel.task_id"))
                == Some(&task.id);
            if isolation.sandbox == expected.sandbox
                && owns_sandbox
                && let Err(error) = manager.remove(&isolation.sandbox).await
            {
                cleanup_error = Some(error);
            }
            expected_worktree
        };

        let worktree_matches = isolation
            .worktree
            .as_deref()
            .is_none_or(|worktree_id| worktree_id == task.id);
        if worktree_matches
            && expected_worktree.exists()
            && let Err(error) = Self::remove_checkout(&expected_worktree)
        {
            cleanup_error.get_or_insert(error);
        }
        cleanup_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn review_diff_includes_untracked_files() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        VmTaskExecutor::git(repo, &["init"]).unwrap();
        VmTaskExecutor::git(repo, &["config", "user.name", "AgentKernel Test"]).unwrap();
        VmTaskExecutor::git(repo, &["config", "user.email", "test@agentkernel.dev"]).unwrap();
        VmTaskExecutor::git(repo, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(repo.join("tracked.txt"), "before\n").unwrap();
        VmTaskExecutor::git(repo, &["add", "tracked.txt"]).unwrap();
        VmTaskExecutor::git(repo, &["commit", "-m", "base"]).unwrap();
        let base_ref = VmTaskExecutor::git(repo, &["rev-parse", "HEAD"]).unwrap();

        std::fs::write(repo.join("tracked.txt"), "after\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new file\n").unwrap();

        let diff = VmTaskExecutor::review_diff(repo, &base_ref).unwrap();
        assert!(diff.contains("tracked.txt"));
        assert!(diff.contains("new.txt"));
        assert!(diff.contains("new file mode"));
    }

    #[test]
    fn task_checkout_has_self_contained_git_metadata() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("source");
        std::fs::create_dir(&repo).unwrap();
        VmTaskExecutor::git(&repo, &["init"]).unwrap();
        VmTaskExecutor::git(&repo, &["config", "user.name", "AgentKernel Test"]).unwrap();
        VmTaskExecutor::git(&repo, &["config", "user.email", "test@agentkernel.dev"]).unwrap();
        VmTaskExecutor::git(&repo, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(repo.join("tracked.txt"), "base\n").unwrap();
        VmTaskExecutor::git(&repo, &["add", "tracked.txt"]).unwrap();
        VmTaskExecutor::git(&repo, &["commit", "-m", "base"]).unwrap();
        let base_ref = VmTaskExecutor::git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let checkout = temp.path().join("task-checkout");

        VmTaskExecutor::create_checkout(&repo, &checkout, "agentkernel/task/test", &base_ref)
            .unwrap();

        assert!(checkout.join(".git").is_dir());
        assert_eq!(
            VmTaskExecutor::git(&checkout, &["branch", "--show-current"]).unwrap(),
            "agentkernel/task/test"
        );
        assert!(VmTaskExecutor::git(&checkout, &["status", "--short"]).is_ok());
    }
}
