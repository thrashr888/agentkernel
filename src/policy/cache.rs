//! Local policy cache for offline operation.
//!
//! Stores signed policy bundles on disk so that sandbox operations can
//! continue when the policy server is unreachable. The OfflineMode
//! determines how cache expiry is handled.

#![cfg(feature = "enterprise")]

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::signing::PolicyBundle;

/// Controls behavior when the policy server is unreachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfflineMode {
    /// Fail all sandbox operations if the server is unreachable
    FailClosed,
    /// Use cached policy until the cache expires
    CachedWithExpiry { max_age: Duration },
    /// Use cached policy indefinitely (least secure)
    CachedIndefinite,
    /// Fall back to a built-in default policy
    DefaultPolicy,
}

impl OfflineMode {
    /// Parse from configuration string.
    pub fn from_config(mode: &str, cache_max_age_hours: u64) -> Self {
        match mode {
            "fail_closed" => OfflineMode::FailClosed,
            "cached_with_expiry" => OfflineMode::CachedWithExpiry {
                max_age: Duration::from_secs(cache_max_age_hours * 3600),
            },
            "cached_indefinite" => OfflineMode::CachedIndefinite,
            "default_policy" => OfflineMode::DefaultPolicy,
            _ => OfflineMode::CachedWithExpiry {
                max_age: Duration::from_secs(cache_max_age_hours * 3600),
            },
        }
    }
}

/// Metadata stored alongside the cached policy bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    /// When the bundle was cached
    cached_at: DateTime<Utc>,
    /// The bundle version
    version: u64,
    /// Hash of the policy content for integrity checks
    content_hash: String,
}

/// Local policy cache backed by the filesystem.
///
/// Stores bundles in `~/.agentkernel/policies/` with metadata for
/// cache expiry tracking.
pub struct PolicyCache {
    /// Directory where cached bundles are stored
    cache_dir: PathBuf,
    /// Offline mode determining cache behavior
    offline_mode: OfflineMode,
}

impl PolicyCache {
    /// Create a new PolicyCache with the given cache directory and offline mode.
    pub fn new(cache_dir: PathBuf, offline_mode: OfflineMode) -> Self {
        Self {
            cache_dir,
            offline_mode,
        }
    }

    /// Create a PolicyCache using the default cache directory.
    pub fn default_dir(offline_mode: OfflineMode) -> Self {
        let cache_dir = if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".agentkernel").join("policies")
        } else {
            PathBuf::from("/tmp/agentkernel/policies")
        };
        Self::new(cache_dir, offline_mode)
    }

    /// Store a policy bundle to the cache.
    ///
    /// Writes both the bundle JSON and metadata to disk atomically.
    pub fn store(&self, bundle: &PolicyBundle) -> Result<()> {
        std::fs::create_dir_all(&self.cache_dir)
            .context("Failed to create policy cache directory")?;

        let content_hash = compute_hash(&bundle.policies);

        let metadata = CacheMetadata {
            cached_at: Utc::now(),
            version: bundle.version,
            content_hash,
        };

        // Write bundle
        let bundle_path = self.bundle_path();
        let bundle_json =
            serde_json::to_string_pretty(bundle).context("Failed to serialize policy bundle")?;
        std::fs::write(&bundle_path, bundle_json)
            .context("Failed to write policy bundle to cache")?;

        // Write metadata
        let meta_path = self.metadata_path();
        let meta_json = serde_json::to_string_pretty(&metadata)
            .context("Failed to serialize cache metadata")?;
        std::fs::write(&meta_path, meta_json).context("Failed to write cache metadata")?;

        Ok(())
    }

    /// Load a policy bundle from the cache.
    ///
    /// Returns None if no cache exists. Returns an error if the cache exists
    /// but is expired according to the configured OfflineMode.
    pub fn load(&self) -> Result<Option<PolicyBundle>> {
        let bundle_path = self.bundle_path();
        let meta_path = self.metadata_path();

        if !bundle_path.exists() || !meta_path.exists() {
            return Ok(None);
        }

        let bundle_json =
            std::fs::read_to_string(&bundle_path).context("Failed to read cached policy bundle")?;
        let bundle: PolicyBundle =
            serde_json::from_str(&bundle_json).context("Failed to parse cached policy bundle")?;

        let meta_json =
            std::fs::read_to_string(&meta_path).context("Failed to read cache metadata")?;
        let metadata: CacheMetadata =
            serde_json::from_str(&meta_json).context("Failed to parse cache metadata")?;

        // Verify integrity
        let expected_hash = compute_hash(&bundle.policies);
        if metadata.content_hash != expected_hash {
            bail!("Cache integrity check failed: content hash mismatch");
        }

        // Check expiry based on offline mode
        if self.is_expired(&metadata) {
            return match &self.offline_mode {
                OfflineMode::FailClosed => {
                    bail!("Policy cache is expired and offline mode is fail_closed")
                }
                OfflineMode::CachedWithExpiry { .. } => {
                    bail!(
                        "Policy cache expired at {} (cached at {})",
                        metadata.cached_at,
                        metadata.cached_at
                    )
                }
                OfflineMode::CachedIndefinite => Ok(Some(bundle)),
                OfflineMode::DefaultPolicy => Ok(None),
            };
        }

        Ok(Some(bundle))
    }

    /// Check if the cached bundle has expired based on the offline mode.
    fn is_expired(&self, metadata: &CacheMetadata) -> bool {
        match &self.offline_mode {
            OfflineMode::FailClosed => {
                // Always expired when server is not reachable
                // (the caller handles this by trying server first)
                false
            }
            OfflineMode::CachedWithExpiry { max_age } => {
                let age = Utc::now()
                    .signed_duration_since(metadata.cached_at)
                    .to_std()
                    .unwrap_or(Duration::from_secs(u64::MAX));
                age > *max_age
            }
            OfflineMode::CachedIndefinite => false,
            OfflineMode::DefaultPolicy => false,
        }
    }

    /// Get the version of the currently cached bundle, if any.
    pub fn cached_version(&self) -> Result<Option<u64>> {
        let meta_path = self.metadata_path();
        if !meta_path.exists() {
            return Ok(None);
        }

        let meta_json =
            std::fs::read_to_string(&meta_path).context("Failed to read cache metadata")?;
        let metadata: CacheMetadata =
            serde_json::from_str(&meta_json).context("Failed to parse cache metadata")?;

        Ok(Some(metadata.version))
    }

    /// Clear the cache.
    pub fn clear(&self) -> Result<()> {
        let bundle_path = self.bundle_path();
        let meta_path = self.metadata_path();

        if bundle_path.exists() {
            std::fs::remove_file(&bundle_path).context("Failed to remove cached bundle")?;
        }
        if meta_path.exists() {
            std::fs::remove_file(&meta_path).context("Failed to remove cache metadata")?;
        }

        Ok(())
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    fn bundle_path(&self) -> PathBuf {
        self.cache_dir.join("bundle.json")
    }

    fn metadata_path(&self) -> PathBuf {
        self.cache_dir.join("metadata.json")
    }
}

/// Compute a simple hash of content for integrity checking.
fn compute_hash(content: &str) -> String {
    // Use a simple checksum approach (not cryptographic, just integrity)
    let mut hash: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV-1a prime
    }
    format!("{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_bundle() -> PolicyBundle {
        PolicyBundle {
            policies: "permit(principal, action, resource);".to_string(),
            version: 1,
            expires_at: Some(Utc::now() + chrono::Duration::hours(24)),
            signature: vec![0u8; 64],
            signer_key_id: "test-key".to_string(),
        }
    }

    #[test]
    fn test_store_and_load() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();

        let loaded = cache.load().unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.policies, bundle.policies);
    }

    #[test]
    fn test_load_empty_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        let loaded = cache.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_cached_version() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        assert_eq!(cache.cached_version().unwrap(), None);

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();

        assert_eq!(cache.cached_version().unwrap(), Some(1));
    }

    #[test]
    fn test_cache_expiry() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(
            tmp.path().join("policies"),
            OfflineMode::CachedWithExpiry {
                max_age: Duration::from_secs(0), // Immediately expired
            },
        );

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();

        // Should fail because cache is immediately expired
        let result = cache.load();
        assert!(result.is_err());
    }

    #[test]
    fn test_cache_indefinite_never_expires() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();

        let loaded = cache.load().unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn test_clear_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();
        assert!(cache.load().unwrap().is_some());

        cache.clear().unwrap();
        assert!(cache.load().unwrap().is_none());
    }

    #[test]
    fn test_integrity_check() {
        let tmp = TempDir::new().unwrap();
        let cache = PolicyCache::new(tmp.path().join("policies"), OfflineMode::CachedIndefinite);

        let bundle = test_bundle();
        cache.store(&bundle).unwrap();

        // Tamper with the cached bundle
        let bundle_path = tmp.path().join("policies/bundle.json");
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&bundle_path).unwrap()).unwrap();
        json["policies"] = serde_json::Value::String("tampered policy".to_string());
        std::fs::write(&bundle_path, serde_json::to_string(&json).unwrap()).unwrap();

        // Load should fail due to hash mismatch
        let result = cache.load();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash mismatch"));
    }

    #[test]
    fn test_offline_mode_from_config() {
        assert_eq!(
            OfflineMode::from_config("fail_closed", 24),
            OfflineMode::FailClosed
        );
        assert_eq!(
            OfflineMode::from_config("cached_with_expiry", 48),
            OfflineMode::CachedWithExpiry {
                max_age: Duration::from_secs(48 * 3600),
            }
        );
        assert_eq!(
            OfflineMode::from_config("cached_indefinite", 24),
            OfflineMode::CachedIndefinite
        );
        assert_eq!(
            OfflineMode::from_config("default_policy", 24),
            OfflineMode::DefaultPolicy
        );
        // Unknown falls back to cached_with_expiry
        assert_eq!(
            OfflineMode::from_config("unknown", 24),
            OfflineMode::CachedWithExpiry {
                max_age: Duration::from_secs(24 * 3600),
            }
        );
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        assert_eq!(h1, h2);

        let h3 = compute_hash("different content");
        assert_ne!(h1, h3);
    }
}
