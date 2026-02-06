//! Persistent volume management for sandboxes.
//!
//! Volumes provide persistent storage that survives sandbox restarts.
//! Data is stored in `~/.agentkernel/volumes/<slug>/` and can be
//! mounted into sandboxes at specified paths.
//!
//! # Example
//!
//! ```bash
//! # Create a volume
//! agentkernel volume create my-data
//!
//! # Mount in sandbox
//! agentkernel create my-sandbox --volume my-data:/data
//!
//! # Files in /data persist after sandbox is removed
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A persistent volume that can be mounted into sandboxes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    /// Unique slug identifier (e.g., "my-data")
    pub slug: String,
    /// Size limit in bytes (0 = unlimited)
    #[serde(default)]
    pub size_bytes: u64,
    /// When the volume was created (RFC3339)
    pub created_at: String,
    /// When the volume was last used (RFC3339)
    #[serde(default)]
    pub last_used: Option<String>,
    /// Number of times this volume has been mounted
    #[serde(default)]
    pub mount_count: u64,
}

impl Volume {
    /// Create a new volume with the given slug.
    pub fn new(slug: &str) -> Self {
        Self {
            slug: slug.to_string(),
            size_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_used: None,
            mount_count: 0,
        }
    }

    /// Create a new volume with a size limit.
    pub fn with_size(slug: &str, size_bytes: u64) -> Self {
        Self {
            slug: slug.to_string(),
            size_bytes,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_used: None,
            mount_count: 0,
        }
    }

    /// Mark the volume as used (updates last_used and mount_count).
    #[allow(dead_code)]
    pub fn mark_used(&mut self) {
        self.last_used = Some(chrono::Utc::now().to_rfc3339());
        self.mount_count += 1;
    }

    /// Get the actual disk usage of the volume in bytes.
    pub fn disk_usage(&self, volumes_dir: &Path) -> Result<u64> {
        let path = volumes_dir.join(&self.slug);
        if !path.exists() {
            return Ok(0);
        }
        dir_size(&path)
    }

    /// Format size for display.
    pub fn format_size(bytes: u64) -> String {
        if bytes == 0 {
            "unlimited".to_string()
        } else if bytes >= 1024 * 1024 * 1024 {
            format!("{:.1}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        } else if bytes >= 1024 * 1024 {
            format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
        } else if bytes >= 1024 {
            format!("{:.1}KB", bytes as f64 / 1024.0)
        } else {
            format!("{}B", bytes)
        }
    }
}

/// A volume mount specification (volume_slug:container_path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VolumeMount {
    /// Volume slug
    pub slug: String,
    /// Path inside the container
    pub mount_path: String,
    /// Read-only mount
    #[serde(default)]
    pub read_only: bool,
}

impl VolumeMount {
    /// Parse a volume mount from "slug:/path" or "slug:/path:ro" format.
    pub fn parse(spec: &str) -> Result<Self> {
        let parts: Vec<&str> = spec.split(':').collect();
        match parts.len() {
            2 => Ok(Self {
                slug: parts[0].to_string(),
                mount_path: parts[1].to_string(),
                read_only: false,
            }),
            3 if parts[2] == "ro" => Ok(Self {
                slug: parts[0].to_string(),
                mount_path: parts[1].to_string(),
                read_only: true,
            }),
            _ => bail!(
                "Invalid volume mount format: '{}'. Expected 'slug:/path' or 'slug:/path:ro'",
                spec
            ),
        }
    }

    /// Convert to Docker -v argument.
    pub fn to_docker_arg(&self, volumes_dir: &Path) -> String {
        let host_path = volumes_dir.join(&self.slug);
        let suffix = if self.read_only { ":ro" } else { "" };
        format!("{}:{}{}", host_path.display(), self.mount_path, suffix)
    }
}

/// Manages persistent volumes.
pub struct VolumeManager {
    /// Directory where volumes are stored
    volumes_dir: PathBuf,
    /// Directory where volume metadata is stored
    metadata_dir: PathBuf,
    /// Loaded volumes
    volumes: HashMap<String, Volume>,
}

impl VolumeManager {
    /// Create a new volume manager.
    pub fn new() -> Result<Self> {
        let base_dir = Self::base_dir();
        let volumes_dir = base_dir.join("volumes");
        let metadata_dir = base_dir.join("volume-metadata");

        std::fs::create_dir_all(&volumes_dir).context("Failed to create volumes directory")?;
        std::fs::create_dir_all(&metadata_dir)
            .context("Failed to create volume metadata directory")?;

        let mut manager = Self {
            volumes_dir,
            metadata_dir,
            volumes: HashMap::new(),
        };
        manager.load_all()?;
        Ok(manager)
    }

    /// Get the base data directory.
    fn base_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".agentkernel")
        } else {
            PathBuf::from("/tmp/agentkernel")
        }
    }

    /// Get the volumes directory path.
    pub fn volumes_dir(&self) -> &Path {
        &self.volumes_dir
    }

    /// Load all volume metadata from disk.
    fn load_all(&mut self) -> Result<()> {
        if !self.metadata_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(volume) = serde_json::from_str::<Volume>(&content)
            {
                self.volumes.insert(volume.slug.clone(), volume);
            }
        }
        Ok(())
    }

    /// Save volume metadata to disk.
    fn save(&self, volume: &Volume) -> Result<()> {
        let path = self.metadata_dir.join(format!("{}.json", volume.slug));
        let content = serde_json::to_string_pretty(volume)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Create a new volume.
    pub fn create(&mut self, slug: &str, size_bytes: Option<u64>) -> Result<Volume> {
        // Validate slug
        if slug.is_empty() {
            bail!("Volume slug cannot be empty");
        }
        if !slug
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            bail!("Volume slug can only contain alphanumeric characters, hyphens, and underscores");
        }
        if self.volumes.contains_key(slug) {
            bail!("Volume '{}' already exists", slug);
        }

        // Create volume directory
        let volume_path = self.volumes_dir.join(slug);
        std::fs::create_dir_all(&volume_path).context("Failed to create volume directory")?;

        // Create volume metadata
        let volume = if let Some(size) = size_bytes {
            Volume::with_size(slug, size)
        } else {
            Volume::new(slug)
        };

        self.save(&volume)?;
        self.volumes.insert(slug.to_string(), volume.clone());

        Ok(volume)
    }

    /// Get a volume by slug.
    pub fn get(&self, slug: &str) -> Option<&Volume> {
        self.volumes.get(slug)
    }

    /// Get a mutable volume by slug and mark it as used.
    #[allow(dead_code)]
    pub fn get_and_mark_used(&mut self, slug: &str) -> Result<&Volume> {
        // First update in place
        {
            let volume = self
                .volumes
                .get_mut(slug)
                .ok_or_else(|| anyhow::anyhow!("Volume '{}' not found", slug))?;
            volume.mark_used();
        }
        // Then save (requires immutable borrow)
        let volume = self.volumes.get(slug).unwrap();
        self.save(volume)?;
        Ok(self.volumes.get(slug).unwrap())
    }

    /// Check if a volume exists.
    pub fn exists(&self, slug: &str) -> bool {
        self.volumes.contains_key(slug)
    }

    /// List all volumes.
    pub fn list(&self) -> Vec<&Volume> {
        self.volumes.values().collect()
    }

    /// Delete a volume.
    pub fn delete(&mut self, slug: &str) -> Result<()> {
        if !self.volumes.contains_key(slug) {
            bail!("Volume '{}' not found", slug);
        }

        // Remove volume directory
        let volume_path = self.volumes_dir.join(slug);
        if volume_path.exists() {
            std::fs::remove_dir_all(&volume_path).context("Failed to remove volume directory")?;
        }

        // Remove metadata file
        let metadata_path = self.metadata_dir.join(format!("{}.json", slug));
        if metadata_path.exists() {
            std::fs::remove_file(&metadata_path).context("Failed to remove volume metadata")?;
        }

        self.volumes.remove(slug);
        Ok(())
    }

    /// Validate that all volumes in a list of mounts exist.
    pub fn validate_mounts(&self, mounts: &[VolumeMount]) -> Result<()> {
        for mount in mounts {
            if !self.exists(&mount.slug) {
                bail!(
                    "Volume '{}' not found. Create it with: agentkernel volume create {}",
                    mount.slug,
                    mount.slug
                );
            }
        }
        Ok(())
    }

    /// Get Docker volume arguments for a list of mounts.
    #[allow(dead_code)]
    pub fn docker_args(&self, mounts: &[VolumeMount]) -> Vec<String> {
        let mut args = Vec::new();
        for mount in mounts {
            args.push("-v".to_string());
            args.push(mount.to_docker_arg(&self.volumes_dir));
        }
        args
    }
}

/// Calculate the total size of a directory recursively.
fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                total += dir_size(&path)?;
            } else {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}

/// Parse a size string like "2GB", "512MB", "1024KB" into bytes.
pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim().to_uppercase();

    if s.is_empty() {
        return Ok(0);
    }

    let (num_str, multiplier) = if s.ends_with("GB") {
        (&s[..s.len() - 2], 1024 * 1024 * 1024)
    } else if s.ends_with("MB") {
        (&s[..s.len() - 2], 1024 * 1024)
    } else if s.ends_with("KB") {
        (&s[..s.len() - 2], 1024)
    } else if s.ends_with('G') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024)
    } else if s.ends_with('M') {
        (&s[..s.len() - 1], 1024 * 1024)
    } else if s.ends_with('K') {
        (&s[..s.len() - 1], 1024)
    } else if s.ends_with('B') {
        (&s[..s.len() - 1], 1)
    } else {
        (s.as_str(), 1)
    };

    let num: f64 = num_str
        .trim()
        .parse()
        .context(format!("Invalid size number: '{}'", num_str))?;

    Ok((num * multiplier as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_volume_new() {
        let vol = Volume::new("test-vol");
        assert_eq!(vol.slug, "test-vol");
        assert_eq!(vol.size_bytes, 0);
        assert!(vol.last_used.is_none());
        assert_eq!(vol.mount_count, 0);
    }

    #[test]
    fn test_volume_with_size() {
        let vol = Volume::with_size("test-vol", 1024 * 1024 * 1024);
        assert_eq!(vol.size_bytes, 1024 * 1024 * 1024);
    }

    #[test]
    fn test_volume_mark_used() {
        let mut vol = Volume::new("test-vol");
        assert!(vol.last_used.is_none());
        assert_eq!(vol.mount_count, 0);

        vol.mark_used();
        assert!(vol.last_used.is_some());
        assert_eq!(vol.mount_count, 1);

        vol.mark_used();
        assert_eq!(vol.mount_count, 2);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(Volume::format_size(0), "unlimited");
        assert_eq!(Volume::format_size(512), "512B");
        assert_eq!(Volume::format_size(1024), "1.0KB");
        assert_eq!(Volume::format_size(1024 * 1024), "1.0MB");
        assert_eq!(Volume::format_size(1024 * 1024 * 1024), "1.0GB");
        assert_eq!(Volume::format_size(2 * 1024 * 1024 * 1024), "2.0GB");
    }

    #[test]
    fn test_volume_mount_parse() {
        let mount = VolumeMount::parse("my-data:/data").unwrap();
        assert_eq!(mount.slug, "my-data");
        assert_eq!(mount.mount_path, "/data");
        assert!(!mount.read_only);

        let mount_ro = VolumeMount::parse("cache:/cache:ro").unwrap();
        assert_eq!(mount_ro.slug, "cache");
        assert_eq!(mount_ro.mount_path, "/cache");
        assert!(mount_ro.read_only);
    }

    #[test]
    fn test_volume_mount_parse_invalid() {
        assert!(VolumeMount::parse("").is_err());
        assert!(VolumeMount::parse("no-path").is_err());
        assert!(VolumeMount::parse("too:many:parts:here").is_err());
    }

    #[test]
    fn test_volume_mount_to_docker_arg() {
        let mount = VolumeMount {
            slug: "my-data".to_string(),
            mount_path: "/data".to_string(),
            read_only: false,
        };
        let arg = mount.to_docker_arg(Path::new("/home/user/.agentkernel/volumes"));
        assert_eq!(arg, "/home/user/.agentkernel/volumes/my-data:/data");

        let mount_ro = VolumeMount {
            slug: "cache".to_string(),
            mount_path: "/cache".to_string(),
            read_only: true,
        };
        let arg_ro = mount_ro.to_docker_arg(Path::new("/home/user/.agentkernel/volumes"));
        assert_eq!(arg_ro, "/home/user/.agentkernel/volumes/cache:/cache:ro");
    }

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_size("2.5GB").unwrap(),
            (2.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(parse_size("512mb").unwrap(), 512 * 1024 * 1024);
        assert_eq!(parse_size("").unwrap(), 0);
    }

    #[test]
    fn test_parse_size_invalid() {
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1XB").is_err());
    }

    #[test]
    fn test_volume_manager_create_delete() {
        let temp_dir = TempDir::new().unwrap();
        // SAFETY: This is a test running in isolation
        unsafe { std::env::set_var("HOME", temp_dir.path()) };

        let mut manager = VolumeManager::new().unwrap();

        // Create
        let vol = manager.create("test-vol", None).unwrap();
        assert_eq!(vol.slug, "test-vol");
        assert!(manager.exists("test-vol"));
        assert!(
            temp_dir
                .path()
                .join(".agentkernel/volumes/test-vol")
                .exists()
        );

        // List
        let volumes = manager.list();
        assert_eq!(volumes.len(), 1);

        // Delete
        manager.delete("test-vol").unwrap();
        assert!(!manager.exists("test-vol"));
        assert!(
            !temp_dir
                .path()
                .join(".agentkernel/volumes/test-vol")
                .exists()
        );
    }

    #[test]
    fn test_volume_manager_duplicate() {
        let temp_dir = TempDir::new().unwrap();
        // SAFETY: This is a test running in isolation
        unsafe { std::env::set_var("HOME", temp_dir.path()) };

        let mut manager = VolumeManager::new().unwrap();
        manager.create("test-vol", None).unwrap();

        // Creating duplicate should fail
        assert!(manager.create("test-vol", None).is_err());
    }

    #[test]
    fn test_volume_manager_invalid_slug() {
        let temp_dir = TempDir::new().unwrap();
        // SAFETY: This is a test running in isolation
        unsafe { std::env::set_var("HOME", temp_dir.path()) };

        let mut manager = VolumeManager::new().unwrap();

        assert!(manager.create("", None).is_err());
        assert!(manager.create("has spaces", None).is_err());
        assert!(manager.create("has/slash", None).is_err());
    }
}
