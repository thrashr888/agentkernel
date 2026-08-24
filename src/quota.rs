//! Enterprise resource quota accounting.
//!
//! Quotas are deliberately evaluated outside Cedar. Cedar answers the
//! authorization question, while this module owns the mutable, concurrency-
//! sensitive resource accounting question. The HTTP layer holds the quota
//! lock while it holds the VM manager lock and performs the lifecycle change,
//! so concurrent requests cannot both pass a quota check.

use crate::vmm::{SandboxState, VmManager};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A tenant/user identity used for quota accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaSubject {
    pub user_id: String,
    pub org_id: String,
}

use crate::config::{ResourceQuotaConfig, ResourceQuotaLimits};

/// A snapshot of usage for one tenant scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QuotaUsage {
    pub total_sandboxes: u32,
    pub running_sandboxes: u32,
    pub total_vcpus: u32,
    pub total_memory_mb: u64,
}

impl QuotaUsage {
    fn add(&mut self, state: &SandboxState, running: bool) {
        self.total_sandboxes = self.total_sandboxes.saturating_add(1);
        self.total_vcpus = self.total_vcpus.saturating_add(state.vcpus);
        self.total_memory_mb = self.total_memory_mb.saturating_add(state.memory_mb);
        if running {
            self.running_sandboxes = self.running_sandboxes.saturating_add(1);
        }
    }

    fn subtract(&mut self, state: &SandboxState, running: bool) {
        self.total_sandboxes = self.total_sandboxes.saturating_sub(1);
        self.total_vcpus = self.total_vcpus.saturating_sub(state.vcpus);
        self.total_memory_mb = self.total_memory_mb.saturating_sub(state.memory_mb);
        if running {
            self.running_sandboxes = self.running_sandboxes.saturating_sub(1);
        }
    }
}

/// A scope's configured limit and current usage.
#[derive(Debug, Clone, Serialize)]
pub struct QuotaScopeStatus {
    pub id: String,
    pub limits: ResourceQuotaLimits,
    pub usage: QuotaUsage,
}

/// Tenant-scoped quota status returned by the API and desktop dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct QuotaStatus {
    pub enabled: bool,
    pub user: QuotaScopeStatus,
    pub organization: QuotaScopeStatus,
}

/// A single quota rejection with actionable resource details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaViolation {
    pub scope: &'static str,
    pub dimension: &'static str,
    pub limit: u64,
    pub current: u64,
    pub requested: u64,
}

impl fmt::Display for QuotaViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{scope} quota exceeded for {dimension}: current {current}, requested {requested}, limit {limit}",
            scope = self.scope,
            dimension = self.dimension,
            current = self.current,
            requested = self.requested,
            limit = self.limit
        )
    }
}

impl std::error::Error for QuotaViolation {}

/// In-memory coordinator for quota checks. The caller must hold this value's
/// outer async mutex for the entire check + lifecycle mutation.
#[derive(Debug, Clone)]
pub struct QuotaController {
    config: ResourceQuotaConfig,
}

impl QuotaController {
    pub fn new(config: ResourceQuotaConfig) -> Self {
        Self { config }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn user_limits(&self, subject: &QuotaSubject) -> ResourceQuotaLimits {
        self.config
            .users
            .get(&subject.user_id)
            .map(|limits| limits.with_fallback(&self.config.default_limits))
            .unwrap_or_else(|| self.config.default_limits.clone())
    }

    fn organization_limits(&self, subject: &QuotaSubject) -> ResourceQuotaLimits {
        self.config
            .organizations
            .get(&subject.org_id)
            .cloned()
            .unwrap_or_default()
    }

    fn owner_ids(state: &SandboxState) -> (&str, &str) {
        (
            state.owner_user_id.as_deref().unwrap_or("anonymous"),
            state.owner_org_id.as_deref().unwrap_or("default"),
        )
    }

    fn usage_for(&self, manager: &VmManager, subject: &QuotaSubject) -> (QuotaUsage, QuotaUsage) {
        let mut user = QuotaUsage::default();
        let mut organization = QuotaUsage::default();
        for (name, running, _) in manager.list() {
            let Some(state) = manager.get_state(name) else {
                continue;
            };
            let (owner_user, owner_org) = Self::owner_ids(state);
            if owner_org == subject.org_id {
                organization.add(state, running);
                if owner_user == subject.user_id {
                    user.add(state, running);
                }
            }
        }
        (user, organization)
    }

    /// Return tenant-isolated usage and limits for dashboard/API display.
    pub fn status(&self, manager: &VmManager, subject: &QuotaSubject) -> QuotaStatus {
        let (user_usage, org_usage) = self.usage_for(manager, subject);
        QuotaStatus {
            enabled: self.enabled(),
            user: QuotaScopeStatus {
                id: subject.user_id.clone(),
                limits: self.user_limits(subject),
                usage: user_usage,
            },
            organization: QuotaScopeStatus {
                id: subject.org_id.clone(),
                limits: self.organization_limits(subject),
                usage: org_usage,
            },
        }
    }

    fn check_scope(
        scope: &'static str,
        limits: &ResourceQuotaLimits,
        usage: &QuotaUsage,
        additional_running: u32,
        additional_sandboxes: u32,
        additional_vcpus: u32,
        additional_memory_mb: u64,
    ) -> Result<()> {
        let checks = [
            (
                limits.max_running_sandboxes.map(u64::from),
                u64::from(usage.running_sandboxes),
                u64::from(additional_running),
                "max_running_sandboxes",
            ),
            (
                limits.max_total_sandboxes.map(u64::from),
                u64::from(usage.total_sandboxes),
                u64::from(additional_sandboxes),
                "max_total_sandboxes",
            ),
            (
                limits.max_total_vcpus.map(u64::from),
                u64::from(usage.total_vcpus),
                u64::from(additional_vcpus),
                "max_total_vcpus",
            ),
            (
                limits.max_total_memory_mb,
                usage.total_memory_mb,
                additional_memory_mb,
                "max_total_memory_mb",
            ),
        ];
        for (limit, current, requested, dimension) in checks {
            if let Some(limit) = limit
                && current.saturating_add(requested) > limit
            {
                return Err(QuotaViolation {
                    scope,
                    dimension,
                    limit,
                    current,
                    requested,
                }
                .into());
            }
        }
        Ok(())
    }

    fn check_with_usage(
        &self,
        manager: &VmManager,
        subject: &QuotaSubject,
        additional_running: u32,
        additional_sandboxes: u32,
        additional_vcpus: u32,
        additional_memory_mb: u64,
    ) -> Result<()> {
        if !self.enabled() {
            return Ok(());
        }
        self.validate_enabled()?;
        let (user_usage, org_usage) = self.usage_for(manager, subject);
        Self::check_scope(
            "user",
            &self.user_limits(subject),
            &user_usage,
            additional_running,
            additional_sandboxes,
            additional_vcpus,
            additional_memory_mb,
        )?;
        Self::check_scope(
            "organization",
            &self.organization_limits(subject),
            &org_usage,
            additional_running,
            additional_sandboxes,
            additional_vcpus,
            additional_memory_mb,
        )?;
        Ok(())
    }

    /// Check the resources for a create operation. HTTP create starts the
    /// sandbox immediately, so the running quota is included.
    pub fn check_create(
        &self,
        manager: &VmManager,
        subject: &QuotaSubject,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<()> {
        self.check_with_usage(manager, subject, 1, 1, vcpus, memory_mb)
    }

    /// Check a persistent sandbox that is created stopped, as with snapshot
    /// restore. It consumes total capacity immediately but does not consume a
    /// running slot until the caller starts it.
    pub fn check_create_stopped(
        &self,
        manager: &VmManager,
        subject: &QuotaSubject,
        vcpus: u32,
        memory_mb: u64,
    ) -> Result<()> {
        self.check_with_usage(manager, subject, 0, 1, vcpus, memory_mb)
    }

    /// Check a stopped sandbox before starting it.
    pub fn check_start(&self, manager: &VmManager, name: &str) -> Result<()> {
        if !self.enabled() {
            return Ok(());
        }
        self.validate_enabled()?;
        let Some(state) = manager.get_state(name) else {
            return Ok(());
        };
        let (owner_user, owner_org) = Self::owner_ids(state);
        let subject = QuotaSubject {
            user_id: owner_user.to_string(),
            org_id: owner_org.to_string(),
        };
        let (user_usage, org_usage) = self.usage_for(manager, &subject);
        // Recovery may already own a confirmed-live runtime while its
        // persisted state still says paused. Rechecking start must preserve
        // that existing slot rather than charging a second one and making
        // metadata reconciliation impossible at the quota limit.
        let additional_running = u32::from(!manager.is_running(name));
        Self::check_scope(
            "user",
            &self.user_limits(&subject),
            &user_usage,
            additional_running,
            0,
            0,
            0,
        )?;
        Self::check_scope(
            "organization",
            &self.organization_limits(&subject),
            &org_usage,
            additional_running,
            0,
            0,
            0,
        )
    }

    /// Check a resource resize while preserving the existing sandbox's slot.
    pub fn check_resize(
        &self,
        manager: &VmManager,
        name: &str,
        new_vcpus: u32,
        new_memory_mb: u64,
    ) -> Result<()> {
        if !self.enabled() {
            return Ok(());
        }
        self.validate_enabled()?;
        let Some(state) = manager.get_state(name) else {
            return Ok(());
        };
        let (owner_user, owner_org) = Self::owner_ids(state);
        let subject = QuotaSubject {
            user_id: owner_user.to_string(),
            org_id: owner_org.to_string(),
        };
        let (mut user_usage, mut org_usage) = self.usage_for(manager, &subject);
        let running = manager.is_running(name);
        user_usage.subtract(state, running);
        org_usage.subtract(state, running);
        Self::check_scope(
            "user",
            &self.user_limits(&subject),
            &user_usage,
            u32::from(running),
            1,
            new_vcpus,
            new_memory_mb,
        )?;
        Self::check_scope(
            "organization",
            &self.organization_limits(&subject),
            &org_usage,
            u32::from(running),
            1,
            new_vcpus,
            new_memory_mb,
        )
    }

    /// Validate configuration and fail closed when enforcement was requested
    /// with invalid limits.
    pub fn validate_enabled(&self) -> Result<()> {
        let warnings = self.config.validate();
        if self.config.enabled
            && warnings.iter().any(|warning| {
                warning.contains("exceeds max_total_sandboxes") || warning.contains("empty")
            })
        {
            bail!(
                "Invalid enterprise resource quota configuration: {}",
                warnings.join("; ")
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendType;
    use tempfile::tempdir;

    fn state(name: &str, user: &str, org: &str, vcpus: u32, memory_mb: u64) -> SandboxState {
        SandboxState {
            name: name.to_string(),
            uuid: name.to_string(),
            image: "alpine:3.24".to_string(),
            vcpus,
            memory_mb,
            vsock_cid: 3,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            backend: Some(BackendType::Docker),
            remote_id: None,
            remote_namespace: None,
            remote_metadata: std::collections::HashMap::new(),
            workspace_revision: None,
            endpoints: Vec::new(),
            work_dir: None,
            container_work_dir: None,
            git_worktree: None,
            config_path: None,
            tenant_id: None,
            ttl_seconds: None,
            expires_at: None,
            ports: Vec::new(),
            managed_network: None,
            ssh_enabled: false,
            ssh_host_port: None,
            volumes: Vec::new(),
            agent: None,
            secret_bindings: Vec::new(),
            secret_mappings: std::collections::HashMap::new(),
            secret_files: Vec::new(),
            placeholder_secrets: false,
            proxy_port: None,
            init_script: None,
            environment: Vec::new(),
            post_create_commands: Vec::new(),
            post_create_completed: false,
            created_from_template: None,
            template_help_text: None,
            labels: std::collections::HashMap::new(),
            description: None,
            last_activity_at: None,
            archived_at: None,
            archived_reason: None,
            dormant_at: None,
            dormant_reason: None,
            lifecycle_policy: None,
            full_state_checkpoint: None,
            full_state_cleanup_pending: Vec::new(),
            full_state_lineage: false,
            paused_at: None,
            forked_from: None,
            firecracker_rootfs: None,
            owner_user_id: Some(user.to_string()),
            owner_org_id: Some(org.to_string()),
        }
    }

    #[test]
    fn quota_limits_parse_and_validate() {
        let config: ResourceQuotaConfig = toml::from_str(
            r#"
                enabled = true
                [default]
                max_total_sandboxes = 4
                max_total_vcpus = 8
                [organizations.acme]
                max_running_sandboxes = 2
            "#,
        )
        .unwrap();
        assert!(config.validate().is_empty());
        assert_eq!(config.default_limits.max_total_vcpus, Some(8));
        assert_eq!(config.organizations["acme"].max_running_sandboxes, Some(2));
    }

    #[test]
    fn zero_limits_are_valid_and_deny_new_resources() {
        let limits = ResourceQuotaLimits {
            max_running_sandboxes: Some(0),
            max_total_sandboxes: Some(0),
            max_total_vcpus: Some(0),
            max_total_memory_mb: Some(0),
        };
        let config = ResourceQuotaConfig {
            enabled: true,
            default_limits: limits,
            ..Default::default()
        };
        assert!(config.validate().is_empty());
        let dir = tempdir().unwrap();
        let manager = VmManager::for_tests(dir.path()).unwrap();
        let subject = QuotaSubject {
            user_id: "u1".to_string(),
            org_id: "acme".to_string(),
        };
        assert!(
            QuotaController::new(config)
                .check_create(&manager, &subject, 1, 1)
                .is_err()
        );
    }

    #[test]
    fn usage_is_tenant_isolated() {
        let dir = tempdir().unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(state("a", "u1", "acme", 2, 256));
        manager.insert_state_for_tests(state("b", "u2", "other", 8, 1024));
        let controller = QuotaController::new(ResourceQuotaConfig {
            enabled: true,
            default_limits: ResourceQuotaLimits {
                max_total_sandboxes: Some(2),
                max_total_vcpus: Some(4),
                ..Default::default()
            },
            users: [(
                "u1".to_string(),
                ResourceQuotaLimits {
                    max_running_sandboxes: Some(3),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        let status = controller.status(
            &manager,
            &QuotaSubject {
                user_id: "u1".to_string(),
                org_id: "acme".to_string(),
            },
        );
        assert_eq!(status.user.usage.total_sandboxes, 1);
        assert_eq!(status.user.limits.max_running_sandboxes, Some(3));
        assert_eq!(status.user.limits.max_total_sandboxes, Some(2));
        assert_eq!(status.organization.usage.total_vcpus, 2);
        controller
            .check_create(
                &manager,
                &QuotaSubject {
                    user_id: "u1".to_string(),
                    org_id: "acme".to_string(),
                },
                2,
                128,
            )
            .unwrap();
        assert!(
            controller
                .check_create(
                    &manager,
                    &QuotaSubject {
                        user_id: "u1".to_string(),
                        org_id: "acme".to_string(),
                    },
                    3,
                    128,
                )
                .is_err()
        );
    }

    #[test]
    fn start_and_resize_checks_respect_tenant_limits() {
        let dir = tempdir().unwrap();
        let mut manager = VmManager::for_tests(dir.path()).unwrap();
        manager.insert_state_for_tests(state("a", "u1", "acme", 2, 256));

        let start_controller = QuotaController::new(ResourceQuotaConfig {
            enabled: true,
            organizations: [(
                "acme".to_string(),
                ResourceQuotaLimits {
                    max_running_sandboxes: Some(0),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        assert!(start_controller.check_start(&manager, "a").is_err());

        let resize_controller = QuotaController::new(ResourceQuotaConfig {
            enabled: true,
            default_limits: ResourceQuotaLimits {
                max_total_vcpus: Some(2),
                max_total_memory_mb: Some(512),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(
            resize_controller
                .check_resize(&manager, "a", 3, 256)
                .is_err()
        );
        assert!(
            resize_controller
                .check_resize(&manager, "a", 2, 512)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn serialized_check_and_mutation_allows_only_one_create() {
        let dir = tempdir().unwrap();
        let manager = std::sync::Arc::new(tokio::sync::RwLock::new(
            VmManager::for_tests(dir.path()).unwrap(),
        ));
        let controller = std::sync::Arc::new(tokio::sync::Mutex::new(QuotaController::new(
            ResourceQuotaConfig {
                enabled: true,
                default_limits: ResourceQuotaLimits {
                    max_total_sandboxes: Some(1),
                    ..Default::default()
                },
                ..Default::default()
            },
        )));
        let subject = QuotaSubject {
            user_id: "u1".to_string(),
            org_id: "acme".to_string(),
        };

        let attempt = |name: &'static str,
                       manager: std::sync::Arc<tokio::sync::RwLock<VmManager>>,
                       controller: std::sync::Arc<tokio::sync::Mutex<QuotaController>>,
                       subject: QuotaSubject| async move {
            let quota = controller.lock().await;
            let mut manager = manager.write().await;
            if quota.check_create(&manager, &subject, 1, 512).is_err() {
                return false;
            }
            manager.insert_state_for_tests(state(name, &subject.user_id, &subject.org_id, 1, 512));
            true
        };

        let (first, second) = tokio::join!(
            attempt(
                "first",
                manager.clone(),
                controller.clone(),
                subject.clone()
            ),
            attempt("second", manager.clone(), controller.clone(), subject),
        );
        assert_eq!(u8::from(first) + u8::from(second), 1);
    }
}
