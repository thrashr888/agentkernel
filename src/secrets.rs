//! Secrets vault for managing API keys and credentials.
//!
//! Supports multiple backends:
//! - `env`: Read secrets from environment variables (default for CI)
//! - `file`: Encrypted file storage at `~/.agentkernel/secrets.json`
//! - `keyring`: OS keychain (macOS Keychain, Linux secret-service) — requires `keyring` feature
//!
//! Secrets are auto-injected as env vars into sandboxes when referenced in config.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Secret metadata stored alongside the value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// When the secret was set (RFC3339)
    pub set_at: String,
    /// Which backend stores this secret
    pub backend: String,
}

/// Supported secret storage backends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretBackend {
    Env,
    #[default]
    File,
    Keyring,
}

impl std::fmt::Display for SecretBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretBackend::Env => write!(f, "env"),
            SecretBackend::File => write!(f, "file"),
            SecretBackend::Keyring => write!(f, "keyring"),
        }
    }
}

impl std::str::FromStr for SecretBackend {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "env" => Ok(SecretBackend::Env),
            "file" => Ok(SecretBackend::File),
            "keyring" => Ok(SecretBackend::Keyring),
            other => bail!(
                "Unknown secret backend: '{}'. Use: env, file, keyring",
                other
            ),
        }
    }
}

/// The secrets vault — manages secrets across backends.
pub struct SecretVault {
    backend: SecretBackend,
    store_path: PathBuf,
    key_path: PathBuf,
}

/// File-based secret store format (JSON).
#[derive(Debug, Default, Serialize, Deserialize)]
struct FileStore {
    /// Map of key -> encrypted value payload.
    secrets: BTreeMap<String, String>,
    /// Metadata per key
    metadata: BTreeMap<String, SecretEntry>,
}

const ENCRYPTED_SECRET_VERSION: &str = "v1";
const FILE_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

impl SecretVault {
    pub fn new(backend: SecretBackend) -> Self {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".agentkernel");
        let store_path = data_dir.join("secrets.json");
        let key_path = data_dir.join("secrets.key");
        Self {
            backend,
            store_path,
            key_path,
        }
    }

    /// Get the active backend.
    pub fn backend(&self) -> SecretBackend {
        self.backend
    }

    /// Set a secret value.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        match self.backend {
            SecretBackend::Env => {
                bail!("Cannot set secrets with 'env' backend. Set environment variables directly.");
            }
            SecretBackend::File => self.file_set(key, value),
            SecretBackend::Keyring => {
                bail!(
                    "Keyring backend requires the 'keyring' feature. Use 'file' or 'env' instead."
                );
            }
        }
    }

    /// Get a secret value.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        match self.backend {
            SecretBackend::Env => Ok(std::env::var(key).ok()),
            SecretBackend::File => self.file_get(key),
            SecretBackend::Keyring => {
                bail!("Keyring backend requires the 'keyring' feature.");
            }
        }
    }

    /// List all secret keys (not values).
    pub fn list(&self) -> Result<Vec<(String, SecretEntry)>> {
        match self.backend {
            SecretBackend::Env => {
                // Can't enumerate env vars meaningfully, return empty
                Ok(Vec::new())
            }
            SecretBackend::File => self.file_list(),
            SecretBackend::Keyring => {
                bail!("Keyring backend requires the 'keyring' feature.");
            }
        }
    }

    /// Delete a secret.
    pub fn delete(&self, key: &str) -> Result<()> {
        match self.backend {
            SecretBackend::Env => {
                bail!("Cannot delete secrets with 'env' backend. Unset the environment variable.");
            }
            SecretBackend::File => self.file_delete(key),
            SecretBackend::Keyring => {
                bail!("Keyring backend requires the 'keyring' feature.");
            }
        }
    }

    /// Resolve a list of secret keys to key=value pairs for sandbox injection.
    #[allow(dead_code)]
    pub fn resolve_keys(&self, keys: &[String]) -> Result<Vec<(String, String)>> {
        let mut result = Vec::new();
        for key in keys {
            if let Some(value) = self.get(key)? {
                result.push((key.clone(), value));
            }
        }
        Ok(result)
    }

    // --- File backend implementation ---

    fn load_store(&self) -> Result<FileStore> {
        if !self.store_path.exists() {
            return Ok(FileStore::default());
        }
        let content =
            std::fs::read_to_string(&self.store_path).context("Failed to read secrets file")?;
        serde_json::from_str(&content).context("Failed to parse secrets file")
    }

    fn save_store(&self, store: &FileStore) -> Result<()> {
        crate::secure_fs::write_private_json(&self.store_path, store)
    }

    fn read_file_key(&self) -> Result<[u8; FILE_KEY_LEN]> {
        use base64::Engine;

        let raw = std::fs::read(&self.key_path).context("Failed to read secrets key file")?;
        let key_bytes = if raw.len() == FILE_KEY_LEN {
            raw
        } else {
            let encoded = String::from_utf8(raw).context("Invalid secrets key format")?;
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .context("Failed to decode secrets key")?
        };
        if key_bytes.len() != FILE_KEY_LEN {
            bail!("Invalid secrets key length");
        }
        let mut key = [0u8; FILE_KEY_LEN];
        key.copy_from_slice(&key_bytes);
        Ok(key)
    }

    fn load_or_create_file_key(&self) -> Result<[u8; FILE_KEY_LEN]> {
        use base64::Engine;

        if self.key_path.exists() {
            return self.read_file_key();
        }

        let mut key = [0u8; FILE_KEY_LEN];
        rand::fill(&mut key);
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);

        match crate::secure_fs::create_private_file(&self.key_path, encoded.as_bytes()) {
            Ok(()) => Ok(key),
            Err(e) => {
                let already_exists = e
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::AlreadyExists);
                if already_exists {
                    self.read_file_key()
                } else {
                    Err(e)
                }
            }
        }
    }

    fn encrypt_value(&self, value: &str) -> Result<String> {
        use base64::Engine;
        use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

        let key_bytes = self.load_or_create_file_key()?;
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|_| anyhow::anyhow!("Failed to initialize cipher"))?;
        let key = LessSafeKey::new(unbound);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::fill(&mut nonce_bytes);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut ciphertext = value.as_bytes().to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
            .map_err(|_| anyhow::anyhow!("Failed to encrypt secret"))?;

        let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce_bytes);
        let ciphertext_b64 = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        Ok(format!(
            "{}:{}:{}",
            ENCRYPTED_SECRET_VERSION, nonce_b64, ciphertext_b64
        ))
    }

    fn decode_legacy_base64(encoded: &str) -> Result<String> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("Failed to decode secret")?;
        String::from_utf8(bytes).context("Secret is not valid UTF-8")
    }

    fn decrypt_value(&self, encoded: &str) -> Result<String> {
        use base64::Engine;
        use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

        let prefix = format!("{}:", ENCRYPTED_SECRET_VERSION);
        let Some(rest) = encoded.strip_prefix(&prefix) else {
            return Self::decode_legacy_base64(encoded);
        };

        let (nonce_b64, ciphertext_b64) = rest
            .split_once(':')
            .context("Invalid encrypted secret format")?;
        let nonce_vec = base64::engine::general_purpose::STANDARD
            .decode(nonce_b64)
            .context("Failed to decode encrypted secret nonce")?;
        if nonce_vec.len() != NONCE_LEN {
            bail!("Invalid encrypted secret nonce length");
        }
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&nonce_vec);

        let mut ciphertext = base64::engine::general_purpose::STANDARD
            .decode(ciphertext_b64)
            .context("Failed to decode encrypted secret payload")?;

        let key_bytes = self.load_or_create_file_key()?;
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|_| anyhow::anyhow!("Failed to initialize cipher"))?;
        let key = LessSafeKey::new(unbound);
        let plaintext = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::empty(),
                &mut ciphertext,
            )
            .map_err(|_| anyhow::anyhow!("Failed to decrypt secret"))?;

        String::from_utf8(plaintext.to_vec()).context("Secret is not valid UTF-8")
    }

    fn file_set(&self, key: &str, value: &str) -> Result<()> {
        let mut store = self.load_store()?;
        store
            .secrets
            .insert(key.to_string(), self.encrypt_value(value)?);
        store.metadata.insert(
            key.to_string(),
            SecretEntry {
                set_at: chrono::Utc::now().to_rfc3339(),
                backend: "file".to_string(),
            },
        );
        self.save_store(&store)
    }

    fn file_get(&self, key: &str) -> Result<Option<String>> {
        let store = self.load_store()?;
        match store.secrets.get(key) {
            Some(encoded) => Ok(Some(self.decrypt_value(encoded)?)),
            None => Ok(None),
        }
    }

    fn file_list(&self) -> Result<Vec<(String, SecretEntry)>> {
        let store = self.load_store()?;
        Ok(store.metadata.into_iter().collect())
    }

    fn file_delete(&self, key: &str) -> Result<()> {
        let mut store = self.load_store()?;
        if store.secrets.remove(key).is_none() {
            bail!("Secret '{}' not found", key);
        }
        store.metadata.remove(key);
        self.save_store(&store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vault(dir: &std::path::Path) -> SecretVault {
        SecretVault {
            backend: SecretBackend::File,
            store_path: dir.join("secrets.json"),
            key_path: dir.join("secrets.key"),
        }
    }

    #[test]
    fn test_set_and_get() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("MY_KEY", "my-secret-value").unwrap();
        let val = vault.get("MY_KEY").unwrap();
        assert_eq!(val, Some("my-secret-value".to_string()));
    }

    #[test]
    fn test_get_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        let val = vault.get("NONEXISTENT").unwrap();
        assert_eq!(val, None);
    }

    #[test]
    fn test_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("KEY_A", "value-a").unwrap();
        vault.set("KEY_B", "value-b").unwrap();

        let entries = vault.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "KEY_A");
        assert_eq!(entries[1].0, "KEY_B");
    }

    #[test]
    fn test_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("TO_DELETE", "value").unwrap();
        assert!(vault.get("TO_DELETE").unwrap().is_some());

        vault.delete("TO_DELETE").unwrap();
        assert!(vault.get("TO_DELETE").unwrap().is_none());
    }

    #[test]
    fn test_delete_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        assert!(vault.delete("NOPE").is_err());
    }

    #[test]
    fn test_overwrite() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("KEY", "old").unwrap();
        vault.set("KEY", "new").unwrap();

        let val = vault.get("KEY").unwrap();
        assert_eq!(val, Some("new".to_string()));

        let entries = vault.list().unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_env_backend_get() {
        let vault = SecretVault {
            backend: SecretBackend::Env,
            store_path: PathBuf::from("/dev/null"),
            key_path: PathBuf::from("/dev/null"),
        };

        // HOME is always set
        let val = vault.get("HOME").unwrap();
        assert!(val.is_some());

        let missing = vault.get("AGENTKERNEL_TEST_NONEXISTENT_12345").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_env_backend_set_fails() {
        let vault = SecretVault {
            backend: SecretBackend::Env,
            store_path: PathBuf::from("/dev/null"),
            key_path: PathBuf::from("/dev/null"),
        };
        assert!(vault.set("KEY", "val").is_err());
    }

    #[test]
    fn test_file_backend_encrypts_values() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("MY_KEY", "super-secret").unwrap();
        let raw = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
        assert!(!raw.contains("super-secret"));
        assert!(raw.contains("v1:"));
    }

    #[test]
    fn test_legacy_base64_compat() {
        use base64::Engine;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        let mut secrets = std::collections::BTreeMap::new();
        secrets.insert(
            "LEGACY_KEY".to_string(),
            base64::engine::general_purpose::STANDARD.encode("legacy-value"),
        );
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "LEGACY_KEY".to_string(),
            SecretEntry {
                set_at: chrono::Utc::now().to_rfc3339(),
                backend: "file".to_string(),
            },
        );
        let store = FileStore { secrets, metadata };
        std::fs::write(
            dir.path().join("secrets.json"),
            serde_json::to_string_pretty(&store).unwrap(),
        )
        .unwrap();

        let val = vault.get("LEGACY_KEY").unwrap();
        assert_eq!(val, Some("legacy-value".to_string()));
    }

    #[test]
    fn test_resolve_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = test_vault(dir.path());

        vault.set("FOUND_KEY", "found-value").unwrap();

        let resolved = vault
            .resolve_keys(&["FOUND_KEY".to_string(), "MISSING_KEY".to_string()])
            .unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0],
            ("FOUND_KEY".to_string(), "found-value".to_string())
        );
    }

    #[test]
    fn test_backend_display() {
        assert_eq!(format!("{}", SecretBackend::Env), "env");
        assert_eq!(format!("{}", SecretBackend::File), "file");
        assert_eq!(format!("{}", SecretBackend::Keyring), "keyring");
    }

    #[test]
    fn test_backend_parse() {
        assert_eq!("env".parse::<SecretBackend>().unwrap(), SecretBackend::Env);
        assert_eq!(
            "file".parse::<SecretBackend>().unwrap(),
            SecretBackend::File
        );
        assert!("unknown".parse::<SecretBackend>().is_err());
    }
}
