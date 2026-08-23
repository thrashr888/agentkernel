//! Tenant-scoped model governance for LLM proxy requests.
//!
//! The proxy is deliberately given a resolved policy rather than a tenant
//! identifier from the request.  A caller can choose a model, but cannot
//! choose the tenant whose policy is evaluated.

use crate::config::LlmGovernanceConfig;
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use tokio::sync::RwLock;

const MAX_PROVIDER_LEN: usize = 64;
const MAX_MODEL_LEN: usize = 256;

/// A normalized provider/model pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NormalizedModel {
    pub provider: String,
    pub model: String,
}

/// Why a governance request was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenialReason {
    MissingModel,
    InvalidModel,
    InvalidProvider,
    ModelNotAllowed,
}

impl DenialReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingModel => "model is required",
            Self::InvalidModel => "model is invalid",
            Self::InvalidProvider => "provider is invalid",
            Self::ModelNotAllowed => "model is not allowed for this tenant",
        }
    }
}

/// A model allowlist resolved for one trusted tenant.
#[derive(Debug, Clone)]
pub struct ModelGovernancePolicy {
    tenant_id: String,
    allowlists: BTreeMap<String, BTreeSet<String>>,
}

impl ModelGovernancePolicy {
    /// Validate every tenant policy without selecting a tenant for a proxy.
    pub fn validate_config(config: &LlmGovernanceConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }
        if config.tenants.is_empty() {
            bail!("enabled LLM governance requires at least one tenant model allowlist")
        }
        for tenant_id in config.tenants.keys() {
            Self::from_config(config, tenant_id)?.expect("enabled governance returns a policy");
        }
        Ok(())
    }

    /// Resolve the policy for `tenant_id` from operator configuration.
    ///
    /// A missing tenant or malformed policy is an error. Callers should fail
    /// startup rather than silently disabling an enabled governance policy.
    pub fn from_config(config: &LlmGovernanceConfig, tenant_id: &str) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }

        let tenant_id = normalize_tenant(tenant_id)?;
        let configured = config.tenants.get(&tenant_id).ok_or_else(|| {
            anyhow::anyhow!("LLM governance has no model allowlist for the configured tenant")
        })?;

        let mut allowlists = BTreeMap::new();
        for (provider, models) in configured {
            let provider = normalize_provider(provider).map_err(|reason| {
                anyhow::anyhow!("Invalid LLM governance provider: {}", reason.as_str())
            })?;
            let entry = allowlists.entry(provider).or_insert_with(BTreeSet::new);
            for model in models {
                let model = normalize_model(model).map_err(|reason| {
                    anyhow::anyhow!("Invalid LLM governance model: {}", reason.as_str())
                })?;
                entry.insert(model);
            }
        }

        Ok(Some(Self {
            tenant_id,
            allowlists,
        }))
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn providers(&self) -> impl Iterator<Item = &str> {
        self.allowlists.keys().map(String::as_str)
    }

    /// Normalize and authorize a provider/model pair.
    pub fn authorize(
        &self,
        provider: &str,
        model: Option<&str>,
    ) -> std::result::Result<NormalizedModel, DenialReason> {
        let provider = normalize_provider(provider).map_err(|_| DenialReason::InvalidProvider)?;
        let Some(model) = model else {
            return Err(DenialReason::MissingModel);
        };
        let model = normalize_model(model).map_err(|_| DenialReason::InvalidModel)?;

        if self
            .allowlists
            .get(&provider)
            .is_some_and(|models| models.contains(&model))
        {
            Ok(NormalizedModel { provider, model })
        } else {
            Err(DenialReason::ModelNotAllowed)
        }
    }
}

/// Normalize a provider identifier. Provider identity comes from the proxy's
/// destination registry, not from a request header or body field.
pub fn normalize_provider(value: &str) -> std::result::Result<String, DenialReason> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > MAX_PROVIDER_LEN
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(DenialReason::InvalidProvider);
    }

    // Keep aliases explicit so configuration and observed provider names have
    // one canonical namespace.
    let value = match value.as_str() {
        "google-ai" | "google_generative_language" => "google",
        "azure-openai" | "azure_openai" => "azure",
        _ => value.as_str(),
    };
    Ok(value.to_string())
}

pub fn normalize_model(value: &str) -> std::result::Result<String, DenialReason> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > MAX_MODEL_LEN
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DenialReason::InvalidModel);
    }
    Ok(value)
}

fn normalize_tenant(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("configured tenant identity is invalid")
    }
    Ok(value.to_string())
}

/// A redacted audit record for a denied LLM request.
#[derive(Debug, Clone, Serialize)]
pub struct GovernanceDenialAudit {
    pub timestamp: String,
    pub sandbox: String,
    pub tenant: String,
    pub provider: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub reason: DenialReason,
}

/// Bounded in-memory governance audit. Values contain normalized metadata only;
/// request bodies and credentials are never retained.
pub static GOVERNANCE_DENIALS: LazyLock<RwLock<Vec<GovernanceDenialAudit>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub async fn record_denial(audit: GovernanceDenialAudit) {
    let mut entries = GOVERNANCE_DENIALS.write().await;
    entries.push(audit);
    const MAX_ENTRIES: usize = 1024;
    if entries.len() > MAX_ENTRIES {
        let remove = entries.len() - MAX_ENTRIES;
        entries.drain(..remove);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LlmGovernanceConfig {
        let mut tenants = BTreeMap::new();
        let mut providers = BTreeMap::new();
        providers.insert(
            "OpenAI".to_string(),
            ["GPT-4o", "gpt-4o-mini"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        providers.insert(
            "anthropic".to_string(),
            ["claude-3-5-sonnet"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        tenants.insert("acme".to_string(), providers);
        LlmGovernanceConfig {
            enabled: true,
            tenants,
        }
    }

    #[test]
    fn allows_normalized_model_for_tenant() {
        let policy = ModelGovernancePolicy::from_config(&config(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(
            policy.authorize("OPENAI", Some(" GPT-4O ")).unwrap(),
            NormalizedModel {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string()
            }
        );
    }

    #[test]
    fn denies_unknown_model() {
        let policy = ModelGovernancePolicy::from_config(&config(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(
            policy.authorize("openai", Some("gpt-5")),
            Err(DenialReason::ModelNotAllowed)
        );
    }

    #[test]
    fn denies_missing_and_unparseable_models() {
        let policy = ModelGovernancePolicy::from_config(&config(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(
            policy.authorize("openai", None),
            Err(DenialReason::MissingModel)
        );
        assert_eq!(
            policy.authorize("openai", Some("\u{7}")),
            Err(DenialReason::InvalidModel)
        );
    }

    #[test]
    fn denies_cross_provider_and_cross_tenant() {
        let policy = ModelGovernancePolicy::from_config(&config(), "acme")
            .unwrap()
            .unwrap();
        assert_eq!(
            policy.authorize("anthropic", Some("gpt-4o")),
            Err(DenialReason::ModelNotAllowed)
        );
        assert!(ModelGovernancePolicy::from_config(&config(), "other").is_err());
    }

    #[test]
    fn disabled_governance_is_a_noop() {
        let mut config = config();
        config.enabled = false;
        assert!(
            ModelGovernancePolicy::from_config(&config, "not-used")
                .unwrap()
                .is_none()
        );
    }
}
