//! VSOCK-based secrets injection (file-based).
//!
//! Injects secrets as files inside the sandbox at `/run/agentkernel/secrets/KEY`.
//! Each secret is written as a separate file with restricted permissions.
//! The sandbox can read secrets from the filesystem without them appearing
//! in environment variables or process listings.
//!
//! ## Placeholder token mode
//!
//! When using `inject_secrets_as_placeholders`, the sandbox receives random
//! placeholder tokens instead of real secret values. The real values never
//! enter the VM. The proxy intercepts outbound traffic and substitutes
//! placeholders with real values in HTTP headers and request bodies.

use crate::backend::{FileInjection, Sandbox};
use anyhow::{Result, bail};
use std::collections::HashMap;

/// Default mount path for secret files inside the sandbox.
pub const DEFAULT_SECRETS_PATH: &str = "/run/agentkernel/secrets";

/// Prefix for placeholder tokens. Easily greppable, never collides with real values.
pub const PLACEHOLDER_PREFIX: &str = "AGENTKERNEL_PLACEHOLDER_";

/// A mapping of placeholder tokens to their real secret values.
/// Shared with the proxy for substitution.
#[derive(Debug, Clone, Default)]
pub struct PlaceholderMap {
    /// placeholder_token -> real_value
    pub mappings: HashMap<String, String>,
}

impl PlaceholderMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a random placeholder token for a secret value.
    /// Returns the placeholder string.
    pub fn insert_secret(&mut self, real_value: &str) -> String {
        let mut bytes = [0u8; 16];
        rand::fill(&mut bytes);
        let token = format!("{}{}", PLACEHOLDER_PREFIX, hex::encode(bytes));
        self.mappings.insert(token.clone(), real_value.to_string());
        token
    }

    /// Replace all placeholder tokens found in a string with their real values.
    /// Returns the substituted string and whether any replacement was made.
    pub fn substitute(&self, input: &str) -> (String, bool) {
        let mut result = input.to_string();
        let mut replaced = false;
        for (placeholder, real_value) in &self.mappings {
            if result.contains(placeholder.as_str()) {
                result = result.replace(placeholder.as_str(), real_value);
                replaced = true;
            }
        }
        (result, replaced)
    }

    /// Replace all placeholder tokens found in a byte slice.
    /// Returns the substituted bytes and whether any replacement was made.
    pub fn substitute_bytes(&self, input: &[u8]) -> (Vec<u8>, bool) {
        // Fast path: check if any placeholder prefix exists
        if !contains_bytes(input, PLACEHOLDER_PREFIX.as_bytes()) {
            return (input.to_vec(), false);
        }
        // Convert to string for replacement (placeholders are always ASCII)
        match std::str::from_utf8(input) {
            Ok(s) => {
                let (result, replaced) = self.substitute(s);
                (result.into_bytes(), replaced)
            }
            Err(_) => (input.to_vec(), false),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
}

/// Check if a byte slice contains a subsequence.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Inject secrets as files into a running sandbox.
///
/// `resolved_secrets` maps key names to their values (pre-resolved from vault).
/// Each secret is written to `{mount_path}/{KEY}` with restricted permissions.
/// Returns the list of keys that were successfully injected.
pub async fn inject_secrets_as_files(
    sandbox: &mut dyn Sandbox,
    mount_path: &str,
    resolved_secrets: &HashMap<String, String>,
) -> Result<Vec<String>> {
    if resolved_secrets.is_empty() {
        return Ok(Vec::new());
    }

    // Validate all key names before writing anything
    for key in resolved_secrets.keys() {
        validate_secret_key(key)?;
    }

    // Create the secrets directory
    sandbox.exec(&["mkdir", "-p", mount_path]).await?;

    let files: Vec<FileInjection> = resolved_secrets
        .iter()
        .map(|(key, value)| FileInjection {
            dest: format!("{}/{}", mount_path, key),
            content: value.as_bytes().to_vec(),
        })
        .collect();

    let injected: Vec<String> = resolved_secrets.keys().cloned().collect();

    sandbox.inject_files(&files).await?;

    // Restrict permissions: only owner can read
    let chmod_cmd = format!("chmod 400 {}/*", mount_path);
    let _ = sandbox.exec(&["sh", "-c", &chmod_cmd]).await;

    Ok(injected)
}

/// Inject secrets as **placeholder tokens** into a running sandbox.
///
/// Instead of injecting real values, generates random placeholder tokens
/// and writes those to the secret files. Returns a `PlaceholderMap` that
/// the proxy uses to substitute real values in outbound traffic.
pub async fn inject_secrets_as_placeholders(
    sandbox: &mut dyn Sandbox,
    mount_path: &str,
    resolved_secrets: &HashMap<String, String>,
) -> Result<(Vec<String>, PlaceholderMap)> {
    if resolved_secrets.is_empty() {
        return Ok((Vec::new(), PlaceholderMap::new()));
    }

    for key in resolved_secrets.keys() {
        validate_secret_key(key)?;
    }

    let mut placeholder_map = PlaceholderMap::new();

    sandbox.exec(&["mkdir", "-p", mount_path]).await?;

    let files: Vec<FileInjection> = resolved_secrets
        .iter()
        .map(|(key, value)| {
            let placeholder = placeholder_map.insert_secret(value);
            FileInjection {
                dest: format!("{}/{}", mount_path, key),
                content: placeholder.into_bytes(),
            }
        })
        .collect();

    let injected: Vec<String> = resolved_secrets.keys().cloned().collect();

    sandbox.inject_files(&files).await?;

    let chmod_cmd = format!("chmod 400 {}/*", mount_path);
    let _ = sandbox.exec(&["sh", "-c", &chmod_cmd]).await;

    Ok((injected, placeholder_map))
}

/// Validate a secret key name (alphanumeric, underscores, hyphens only).
pub fn validate_secret_key(key: &str) -> Result<()> {
    if key.is_empty() {
        bail!("Secret key cannot be empty");
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!(
            "Secret key '{}' contains invalid characters. Use alphanumeric, underscore, or hyphen only.",
            key
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_secret_key_valid() {
        assert!(validate_secret_key("OPENAI_API_KEY").is_ok());
        assert!(validate_secret_key("my-secret").is_ok());
        assert!(validate_secret_key("SECRET123").is_ok());
        assert!(validate_secret_key("a").is_ok());
    }

    #[test]
    fn test_validate_secret_key_invalid() {
        assert!(validate_secret_key("").is_err());
        assert!(validate_secret_key("secret/key").is_err());
        assert!(validate_secret_key("key with spaces").is_err());
    }

    #[test]
    fn test_default_secrets_path() {
        assert_eq!(DEFAULT_SECRETS_PATH, "/run/agentkernel/secrets");
    }

    #[test]
    fn test_placeholder_map_insert_and_substitute() {
        let mut map = PlaceholderMap::new();
        let placeholder = map.insert_secret("sk-real-api-key-12345");

        assert!(placeholder.starts_with(PLACEHOLDER_PREFIX));
        assert_eq!(placeholder.len(), PLACEHOLDER_PREFIX.len() + 32); // 16 bytes = 32 hex chars

        let input = format!("Authorization: Bearer {}", placeholder);
        let (result, replaced) = map.substitute(&input);
        assert!(replaced);
        assert_eq!(result, "Authorization: Bearer sk-real-api-key-12345");
    }

    #[test]
    fn test_placeholder_map_no_match() {
        let map = PlaceholderMap::new();
        let (result, replaced) = map.substitute("no placeholders here");
        assert!(!replaced);
        assert_eq!(result, "no placeholders here");
    }

    #[test]
    fn test_placeholder_map_multiple_secrets() {
        let mut map = PlaceholderMap::new();
        let p1 = map.insert_secret("secret-one");
        let p2 = map.insert_secret("secret-two");

        assert_ne!(p1, p2);

        let input = format!("key1={} key2={}", p1, p2);
        let (result, replaced) = map.substitute(&input);
        assert!(replaced);
        assert_eq!(result, "key1=secret-one key2=secret-two");
    }

    #[test]
    fn test_placeholder_map_substitute_bytes() {
        let mut map = PlaceholderMap::new();
        let placeholder = map.insert_secret("real-value");

        let input = format!("{{\"api_key\":\"{}\"}}", placeholder);
        let (result, replaced) = map.substitute_bytes(input.as_bytes());
        assert!(replaced);
        assert_eq!(
            String::from_utf8(result).unwrap(),
            "{\"api_key\":\"real-value\"}"
        );
    }

    #[test]
    fn test_placeholder_map_substitute_bytes_no_match() {
        let map = PlaceholderMap::new();
        let input = b"no placeholders here";
        let (result, replaced) = map.substitute_bytes(input);
        assert!(!replaced);
        assert_eq!(result, input);
    }

    #[test]
    fn test_placeholder_map_substitute_bytes_invalid_utf8() {
        let mut map = PlaceholderMap::new();
        map.insert_secret("value");
        let input: &[u8] = &[0xFF, 0xFE, 0xFD];
        let (result, replaced) = map.substitute_bytes(input);
        assert!(!replaced);
        assert_eq!(result, input);
    }

    #[test]
    fn test_contains_bytes() {
        assert!(contains_bytes(b"hello world", b"world"));
        assert!(contains_bytes(b"hello world", b"hello"));
        assert!(!contains_bytes(b"hello world", b"xyz"));
        assert!(!contains_bytes(b"hi", b"hello"));
    }
}
