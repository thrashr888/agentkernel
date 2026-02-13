//! VSOCK-based secrets injection (file-based).
//!
//! Injects secrets as files inside the sandbox at `/run/agentkernel/secrets/KEY`.
//! Each secret is written as a separate file with restricted permissions.
//! The sandbox can read secrets from the filesystem without them appearing
//! in environment variables or process listings.

use crate::backend::{FileInjection, Sandbox};
use anyhow::{Result, bail};
use std::collections::HashMap;

/// Default mount path for secret files inside the sandbox.
pub const DEFAULT_SECRETS_PATH: &str = "/run/agentkernel/secrets";

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
}
