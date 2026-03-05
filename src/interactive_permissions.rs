//! Interactive permission model for runtime permission requests.
//!
//! Provides a grant/reject system where tools can request elevated permissions
//! at runtime, and the controlling client (MCP host, HTTP caller, desktop UI)
//! can approve or deny with configurable scope.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// What permission is being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    SandboxRemove,
    SandboxCreate,
    NetworkAccess,
    MountDirectory,
    SudoExec,
    FileDelete,
}

impl std::fmt::Display for PermissionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionKind::SandboxRemove => write!(f, "sandbox_remove"),
            PermissionKind::SandboxCreate => write!(f, "sandbox_create"),
            PermissionKind::NetworkAccess => write!(f, "network_access"),
            PermissionKind::MountDirectory => write!(f, "mount_directory"),
            PermissionKind::SudoExec => write!(f, "sudo_exec"),
            PermissionKind::FileDelete => write!(f, "file_delete"),
        }
    }
}

impl PermissionKind {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sandbox_remove" => Some(Self::SandboxRemove),
            "sandbox_create" => Some(Self::SandboxCreate),
            "network_access" => Some(Self::NetworkAccess),
            "mount_directory" => Some(Self::MountDirectory),
            "sudo_exec" => Some(Self::SudoExec),
            "file_delete" => Some(Self::FileDelete),
            _ => None,
        }
    }

    pub fn description(&self, sandbox: Option<&str>) -> String {
        let target = sandbox.map(|s| format!(" for '{s}'")).unwrap_or_default();
        match self {
            PermissionKind::SandboxRemove => {
                format!("Remove sandbox{target} and all its data")
            }
            PermissionKind::SandboxCreate => {
                format!("Create a new sandbox{target}")
            }
            PermissionKind::NetworkAccess => {
                format!("Enable network access{target}")
            }
            PermissionKind::MountDirectory => {
                format!("Mount a host directory into sandbox{target}")
            }
            PermissionKind::SudoExec => {
                format!("Execute command as root{target}")
            }
            PermissionKind::FileDelete => {
                format!("Delete files{target}")
            }
        }
    }
}

/// Scope of a permission grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantScope {
    /// Single operation, consumed on use.
    Once,
    /// Valid until sandbox stops or session ends.
    Session,
    /// Persisted to disk, survives restarts.
    Always,
}

/// A stored permission grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub id: String,
    pub kind: PermissionKind,
    pub scope: GrantScope,
    /// None = applies to all sandboxes.
    pub sandbox: Option<String>,
    pub granted_at: String,
    pub granted_by: String,
}

/// A pending permission request (returned as structured error metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub kind: PermissionKind,
    pub sandbox: Option<String>,
    pub description: String,
}

/// Store for active permission grants.
pub struct PermissionStore {
    grants: Mutex<Vec<PermissionGrant>>,
    persistent_path: PathBuf,
}

impl Default for PermissionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionStore {
    /// Create a new store, loading any persisted `Always` grants from disk.
    pub fn new() -> Self {
        let persistent_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agentkernel")
            .join("permissions.json");

        let grants = Self::load_from_disk(&persistent_path).unwrap_or_default();

        Self {
            grants: Mutex::new(grants),
            persistent_path,
        }
    }

    /// Check if a permission is currently granted.
    pub fn check(&self, kind: PermissionKind, sandbox: Option<&str>) -> bool {
        let grants = self.grants.lock().unwrap();
        grants
            .iter()
            .any(|g| g.kind == kind && (g.sandbox.is_none() || g.sandbox.as_deref() == sandbox))
    }

    /// Grant a permission. Returns the grant ID.
    pub fn grant(
        &self,
        kind: PermissionKind,
        scope: GrantScope,
        sandbox: Option<String>,
        granted_by: &str,
    ) -> String {
        let id = format!("grant_{}", uuid_v4());
        let grant = PermissionGrant {
            id: id.clone(),
            kind,
            scope,
            sandbox,
            granted_at: chrono::Utc::now().to_rfc3339(),
            granted_by: granted_by.to_string(),
        };

        let mut grants = self.grants.lock().unwrap();
        grants.push(grant);

        if scope == GrantScope::Always {
            let _ = self.save_persistent(&grants);
        }

        id
    }

    /// Revoke a specific grant by ID.
    pub fn revoke(&self, id: &str) -> bool {
        let mut grants = self.grants.lock().unwrap();
        let before = grants.len();
        grants.retain(|g| g.id != id);
        let removed = grants.len() < before;

        if removed {
            let _ = self.save_persistent(&grants);
        }

        removed
    }

    /// List all active grants.
    pub fn list(&self) -> Vec<PermissionGrant> {
        let grants = self.grants.lock().unwrap();
        grants.clone()
    }

    /// Consume a one-time grant (used after successful operation).
    /// Returns true if a matching `Once` grant was found and consumed.
    pub fn consume_once(&self, kind: PermissionKind, sandbox: Option<&str>) -> bool {
        let mut grants = self.grants.lock().unwrap();
        if let Some(pos) = grants.iter().position(|g| {
            g.kind == kind
                && g.scope == GrantScope::Once
                && (g.sandbox.is_none() || g.sandbox.as_deref() == sandbox)
        }) {
            grants.remove(pos);
            true
        } else {
            false
        }
    }

    /// Generate a permission request ID and description for structured error responses.
    pub fn create_request(kind: PermissionKind, sandbox: Option<&str>) -> PermissionRequest {
        PermissionRequest {
            id: format!("req_{}", uuid_v4()),
            kind,
            sandbox: sandbox.map(String::from),
            description: kind.description(sandbox),
        }
    }

    fn load_from_disk(path: &PathBuf) -> Result<Vec<PermissionGrant>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = std::fs::read_to_string(path)?;
        let grants: Vec<PermissionGrant> = serde_json::from_str(&data)?;
        // Only load persistent grants
        Ok(grants
            .into_iter()
            .filter(|g| g.scope == GrantScope::Always)
            .collect())
    }

    fn save_persistent(&self, grants: &[PermissionGrant]) -> Result<()> {
        let persistent: Vec<&PermissionGrant> = grants
            .iter()
            .filter(|g| g.scope == GrantScope::Always)
            .collect();

        if let Some(parent) = self.persistent_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&persistent)?;
        std::fs::write(&self.persistent_path, data)?;
        Ok(())
    }
}

/// Simple UUID v4 generation (no external crate needed).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seed = now.as_nanos();
    format!(
        "{:016x}{:016x}",
        seed,
        seed.wrapping_mul(6364136223846793005)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grant_and_check() {
        let store = PermissionStore::new();
        assert!(!store.check(PermissionKind::SandboxRemove, Some("test")));

        store.grant(
            PermissionKind::SandboxRemove,
            GrantScope::Session,
            Some("test".to_string()),
            "user",
        );
        assert!(store.check(PermissionKind::SandboxRemove, Some("test")));
        assert!(!store.check(PermissionKind::SandboxRemove, Some("other")));
    }

    #[test]
    fn test_wildcard_grant() {
        let store = PermissionStore::new();
        store.grant(
            PermissionKind::SandboxCreate,
            GrantScope::Session,
            None,
            "user",
        );
        // Wildcard grant matches any sandbox
        assert!(store.check(PermissionKind::SandboxCreate, Some("any")));
        assert!(store.check(PermissionKind::SandboxCreate, None));
    }

    #[test]
    fn test_consume_once() {
        let store = PermissionStore::new();
        store.grant(
            PermissionKind::SandboxRemove,
            GrantScope::Once,
            Some("test".to_string()),
            "user",
        );
        assert!(store.check(PermissionKind::SandboxRemove, Some("test")));
        assert!(store.consume_once(PermissionKind::SandboxRemove, Some("test")));
        // Grant should be consumed
        assert!(!store.check(PermissionKind::SandboxRemove, Some("test")));
    }

    #[test]
    fn test_revoke() {
        let store = PermissionStore::new();
        let id = store.grant(PermissionKind::SudoExec, GrantScope::Session, None, "user");
        assert!(store.check(PermissionKind::SudoExec, None));
        assert!(store.revoke(&id));
        assert!(!store.check(PermissionKind::SudoExec, None));
    }

    #[test]
    fn test_list() {
        let store = PermissionStore::new();
        store.grant(
            PermissionKind::SandboxRemove,
            GrantScope::Session,
            None,
            "user",
        );
        store.grant(
            PermissionKind::SandboxCreate,
            GrantScope::Once,
            Some("test".to_string()),
            "user",
        );
        let list = store.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_permission_kind_roundtrip() {
        let kinds = [
            PermissionKind::SandboxRemove,
            PermissionKind::SandboxCreate,
            PermissionKind::NetworkAccess,
            PermissionKind::MountDirectory,
            PermissionKind::SudoExec,
            PermissionKind::FileDelete,
        ];
        for kind in &kinds {
            let s = kind.to_string();
            let parsed = PermissionKind::from_str(&s);
            assert_eq!(parsed, Some(*kind));
        }
    }

    #[test]
    fn test_create_request() {
        let req =
            PermissionStore::create_request(PermissionKind::SandboxRemove, Some("my-sandbox"));
        assert!(req.id.starts_with("req_"));
        assert_eq!(req.kind, PermissionKind::SandboxRemove);
        assert_eq!(req.sandbox.as_deref(), Some("my-sandbox"));
        assert!(req.description.contains("my-sandbox"));
    }
}
