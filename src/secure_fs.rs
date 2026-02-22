//! Secure filesystem helpers for sensitive on-disk data.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Ensure a directory exists and is private on Unix platforms.
pub fn ensure_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("Failed to create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).with_context(
            || format!("Failed to set directory permissions for {}", path.display()),
        )?;
    }
    Ok(())
}

/// Create a file with private permissions and fail if it already exists.
pub fn create_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", path.display()))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set file permissions for {}", path.display()))?;
    }

    Ok(())
}

/// Atomically replace a file with private-permission content.
pub fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow::anyhow!("Path '{}' does not have a parent directory", path.display())
    })?;
    ensure_private_dir(parent)?;

    let tmp_path = temp_path(parent, path)?;
    create_private_file(&tmp_path, bytes)?;

    if let Err(rename_err) = std::fs::rename(&tmp_path, path) {
        #[cfg(windows)]
        {
            if path.exists() {
                let _ = std::fs::remove_file(path);
                std::fs::rename(&tmp_path, path)
                    .with_context(|| format!("Failed to replace {}", path.display()))?;
            } else {
                return Err(rename_err)
                    .with_context(|| format!("Failed to move temp file to {}", path.display()));
            }
        }
        #[cfg(not(windows))]
        {
            return Err(rename_err)
                .with_context(|| format!("Failed to move temp file to {}", path.display()));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set file permissions for {}", path.display()))?;
    }

    Ok(())
}

/// Serialize and atomically write JSON to a private file.
pub fn write_private_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let payload = serde_json::to_vec_pretty(value).context("Failed to serialize JSON")?;
    write_private_file_atomic(path, &payload)
}

fn temp_path(parent: &Path, final_path: &Path) -> Result<PathBuf> {
    let file_name = final_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid file name for '{}'", final_path.display()))?;
    let candidate = parent.join(format!(".{}.tmp-{}", file_name, uuid::Uuid::now_v7()));
    if candidate == final_path {
        bail!("Failed to build temporary file path");
    }
    Ok(candidate)
}
