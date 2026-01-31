//! Enterprise policy engine for centralized authorization management.
//!
//! Ties together Cedar policy evaluation, Ed25519 signature verification,
//! HTTP policy fetching, local caching, and audit logging into a unified
//! PolicyEngine that integrates with the sandbox lifecycle.

pub mod audit;
pub mod cache;
pub mod cedar;
pub mod client;
pub mod signing;
pub mod streaming;
pub mod tenant;

use anyhow::{Result, bail};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};

use crate::config::EnterpriseConfig;

pub use audit::{PolicyAuditLogger, PolicyDecisionLog};
pub use cache::{OfflineMode, PolicyCache};
pub use cedar::{Action, CedarEngine, PolicyDecision, PolicyEffect, Principal, Resource};
pub use client::PolicyClient;
pub use signing::{PolicyBundle, TrustAnchor, verify_bundle};

/// Default Cedar policy used when no remote policies are available
/// and offline_mode is "default_policy".
const DEFAULT_POLICY: &str = r#"
// Default enterprise policy: permit all actions for authenticated users
permit(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
);
"#;

/// The unified policy engine that coordinates all enterprise policy components.
///
/// Handles:
/// - Loading policies from server or cache
/// - Verifying policy bundle signatures
/// - Evaluating authorization requests via Cedar
/// - Logging decisions to the audit trail
/// - Background polling for policy updates
pub struct PolicyEngine {
    /// Cedar evaluation engine (behind RwLock for hot-reload)
    engine: Arc<RwLock<CedarEngine>>,
    /// Policy cache for offline operation
    cache: PolicyCache,
    /// Audit logger
    audit: PolicyAuditLogger,
    /// HTTP client for the policy server
    client: Option<Arc<PolicyClient>>,
    /// Trust anchors for signature verification
    trust_anchors: Vec<TrustAnchor>,
    /// Current policy version
    current_version: Arc<RwLock<u64>>,
    /// Organization ID
    org_id: Option<String>,
    /// Shutdown signal sender
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl PolicyEngine {
    /// Create a new PolicyEngine from enterprise configuration.
    pub fn new(config: &EnterpriseConfig) -> Result<Self> {
        if !config.enabled {
            bail!("Enterprise policy engine is not enabled");
        }

        let offline_mode =
            OfflineMode::from_config(&config.offline_mode, config.cache_max_age_hours);

        let cache = PolicyCache::default_dir(offline_mode.clone());
        let audit = PolicyAuditLogger::default_path();

        // Build trust anchors from config
        let trust_anchors = build_trust_anchors(&config.trust_anchors.keys);

        // Create HTTP client if server is configured
        let client = if let Some(ref server) = config.policy_server {
            let api_key = config
                .api_key_env
                .as_ref()
                .and_then(|env_name| std::env::var(env_name).ok());
            Some(Arc::new(PolicyClient::new(server, api_key)?))
        } else {
            None
        };

        // Try to load policies from cache first
        let initial_policies = match cache.load() {
            Ok(Some(bundle)) => {
                // Verify signature if we have trust anchors
                if !trust_anchors.is_empty() {
                    if let Err(e) = verify_bundle(&bundle, &trust_anchors, None) {
                        eprintln!(
                            "[enterprise] Cached bundle failed verification: {}. Using default.",
                            e
                        );
                        DEFAULT_POLICY.to_string()
                    } else {
                        bundle.policies.clone()
                    }
                } else {
                    bundle.policies.clone()
                }
            }
            Ok(None) => DEFAULT_POLICY.to_string(),
            Err(e) => {
                eprintln!("[enterprise] Failed to load cache: {}. Using default.", e);
                DEFAULT_POLICY.to_string()
            }
        };

        let engine = CedarEngine::new(&initial_policies)?;
        let current_version = cache.cached_version()?.unwrap_or(0);

        Ok(Self {
            engine: Arc::new(RwLock::new(engine)),
            cache,
            audit,
            client,
            trust_anchors,
            current_version: Arc::new(RwLock::new(current_version)),
            org_id: config.org_id.clone(),
            shutdown_tx: None,
        })
    }

    /// Start the policy engine: fetch initial policies and begin polling.
    pub async fn start(&mut self) -> Result<()> {
        // Try to fetch fresh policies from server
        if let Some(ref client) = self.client {
            match client.fetch_bundle().await {
                Ok(bundle) => {
                    self.apply_bundle(bundle).await?;
                }
                Err(e) => {
                    eprintln!(
                        "[enterprise] Could not reach policy server: {}. Using cached/default.",
                        e
                    );
                }
            }
        }

        // Start background polling if client is configured
        if let Some(ref client) = self.client {
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            self.shutdown_tx = Some(shutdown_tx);

            let mut bundle_rx = client.clone().poll(Duration::from_secs(300), shutdown_rx);

            let engine = self.engine.clone();
            let trust_anchors = self.trust_anchors.clone();
            let cache_dir = self.cache.cache_dir().to_path_buf();
            let offline_mode = OfflineMode::from_config("cached_with_expiry", 24);
            let current_version = self.current_version.clone();

            tokio::spawn(async move {
                while bundle_rx.changed().await.is_ok() {
                    // Clone the bundle out of the borrow immediately to avoid
                    // holding the non-Send Ref across an await point.
                    let bundle = { bundle_rx.borrow().clone() };

                    if let Some(bundle) = bundle {
                        // Verify signature
                        let min_ver = *current_version.read().await;
                        if !trust_anchors.is_empty()
                            && let Err(e) = verify_bundle(&bundle, &trust_anchors, Some(min_ver))
                        {
                            eprintln!("[enterprise] Policy bundle verification failed: {}", e);
                            continue;
                        }

                        // Update engine
                        {
                            let mut eng = engine.write().await;
                            if let Err(e) = eng.update_policies(&bundle.policies) {
                                eprintln!("[enterprise] Failed to update policies: {}", e);
                                continue;
                            }
                        }

                        // Update cache
                        let cache = PolicyCache::new(cache_dir.clone(), offline_mode.clone());
                        if let Err(e) = cache.store(&bundle) {
                            eprintln!("[enterprise] Failed to cache bundle: {}", e);
                        }

                        // Update version
                        *current_version.write().await = bundle.version;
                    }
                }
            });
        }

        Ok(())
    }

    /// Evaluate an authorization request.
    ///
    /// Performs Cedar evaluation, logs the decision to the audit trail,
    /// and returns the result.
    pub async fn evaluate(
        &self,
        principal: &Principal,
        action: Action,
        resource: &Resource,
    ) -> PolicyDecision {
        let engine = self.engine.read().await;
        let decision = engine.evaluate(principal, action, resource, None);

        // Log to audit trail
        let log_entry = PolicyDecisionLog::new(
            &principal.id,
            action,
            &resource.name,
            decision.decision,
            decision.matched_policies.clone(),
            decision.evaluation_time_us,
            self.org_id.clone(),
            Some(decision.reason.clone()),
        );

        if let Err(e) = self.audit.log_decision(&log_entry) {
            eprintln!("[enterprise] Failed to write audit log: {}", e);
        }

        decision
    }

    /// Force a policy reload from the server.
    pub async fn reload(&mut self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No policy server configured"))?;

        let bundle = client.fetch_bundle().await?;
        self.apply_bundle(bundle).await
    }

    /// Get the current policy version.
    pub async fn version(&self) -> u64 {
        *self.current_version.read().await
    }

    /// Get a reference to the audit logger.
    pub fn audit_logger(&self) -> &PolicyAuditLogger {
        &self.audit
    }

    /// Apply a fetched bundle: verify, update engine, cache.
    async fn apply_bundle(&self, bundle: PolicyBundle) -> Result<()> {
        // Verify signature if trust anchors are configured
        let min_ver = *self.current_version.read().await;
        if !self.trust_anchors.is_empty() {
            verify_bundle(&bundle, &self.trust_anchors, Some(min_ver))?;
        }

        // Update the Cedar engine
        {
            let mut engine = self.engine.write().await;
            engine.update_policies(&bundle.policies)?;
        }

        // Cache the bundle
        self.cache.store(&bundle)?;

        // Update current version
        *self.current_version.write().await = bundle.version;

        Ok(())
    }

    /// Shut down the policy engine (stop background polling).
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
    }
}

impl Drop for PolicyEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Build trust anchors from key ID strings.
///
/// In production, these would be loaded from a secure key store.
/// For now, the keys in config are treated as identifiers and
/// actual public key material would need to be provisioned separately.
fn build_trust_anchors(key_ids: &[String]) -> Vec<TrustAnchor> {
    key_ids
        .iter()
        .map(|key_id| TrustAnchor {
            key_id: key_id.clone(),
            // Placeholder: in production, resolve actual key material
            public_key: vec![0u8; 32],
            valid_from: chrono::Utc::now() - chrono::Duration::days(365),
            valid_until: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EnterpriseConfig {
        EnterpriseConfig {
            enabled: true,
            policy_server: None, // No server for unit tests
            org_id: Some("test-org".to_string()),
            api_key_env: None,
            offline_mode: "default_policy".to_string(),
            cache_max_age_hours: 24,
            trust_anchors: crate::config::TrustAnchorsConfig { keys: vec![] },
        }
    }

    #[test]
    fn test_engine_creation() {
        let config = test_config();
        let engine = PolicyEngine::new(&config);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_disabled_engine() {
        let mut config = test_config();
        config.enabled = false;
        let result = PolicyEngine::new(&config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_evaluate_default_policy() {
        let config = test_config();
        let engine = PolicyEngine::new(&config).unwrap();

        let principal = Principal {
            id: "alice".to_string(),
            email: "alice@test.com".to_string(),
            org_id: "test-org".to_string(),
            roles: vec!["developer".to_string()],
            mfa_verified: true,
        };

        let resource = Resource {
            name: "test-sandbox".to_string(),
            agent_type: "claude".to_string(),
            runtime: "python".to_string(),
        };

        // Default policy permits all authenticated users
        let decision = engine.evaluate(&principal, Action::Run, &resource).await;
        assert!(decision.is_permit());
    }

    #[tokio::test]
    async fn test_version_tracking() {
        let config = test_config();
        let engine = PolicyEngine::new(&config).unwrap();
        assert_eq!(engine.version().await, 0);
    }

    #[test]
    fn test_build_trust_anchors() {
        let keys = vec!["key1".to_string(), "key2".to_string()];
        let anchors = build_trust_anchors(&keys);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].key_id, "key1");
        assert_eq!(anchors[1].key_id, "key2");
    }
}
