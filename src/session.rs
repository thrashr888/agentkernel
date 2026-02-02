//! Agent sessions — tie sandbox lifecycle to an agent conversation.
//!
//! A session bundles: sandbox name, agent type, audit trail, injected files,
//! env vars, and timestamps. Sessions can be saved and resumed.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Session metadata persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session name (user-facing, e.g. "feature-x")
    pub name: String,
    /// Sandbox name (e.g. "session-feature-x")
    pub sandbox: String,
    /// Agent type (claude, gemini, codex, opencode)
    pub agent: String,
    /// Current status
    pub status: SessionStatus,
    /// When the session was created (RFC3339)
    pub created_at: String,
    /// When the session was last active (RFC3339)
    pub last_active: String,
    /// Number of commands executed in this session
    pub exec_count: u64,
    /// Snapshot name (if session was saved)
    #[serde(default)]
    pub snapshot: Option<String>,
}

/// Session status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Stopped,
    Saved,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Stopped => write!(f, "stopped"),
            SessionStatus::Saved => write!(f, "saved"),
        }
    }
}

/// Directory where session metadata lives.
fn sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/agentkernel/sessions")
}

/// Create a new session.
pub fn create(name: &str, agent: &str) -> Result<Session> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{}.json", name));
    if path.exists() {
        bail!(
            "Session '{}' already exists. Use 'session resume' or 'session delete' first.",
            name
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let session = Session {
        name: name.to_string(),
        sandbox: format!("session-{}", name),
        agent: agent.to_string(),
        status: SessionStatus::Running,
        created_at: now.clone(),
        last_active: now,
        exec_count: 0,
        snapshot: None,
    };

    save_session(&session)?;
    Ok(session)
}

/// Get a session by name.
pub fn get(name: &str) -> Result<Option<Session>> {
    let path = sessions_dir().join(format!("{}.json", name));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let session: Session = serde_json::from_str(&content)?;
    Ok(Some(session))
}

/// List all sessions.
pub fn list() -> Result<Vec<Session>> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            if let Ok(session) = serde_json::from_str::<Session>(&content) {
                sessions.push(session);
            }
        }
    }
    sessions.sort_by(|a, b| a.last_active.cmp(&b.last_active));
    Ok(sessions)
}

/// Update session metadata (bump last_active and exec_count).
#[allow(dead_code)]
pub fn record_exec(name: &str) -> Result<()> {
    let mut session = get(name)?.ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;
    session.last_active = chrono::Utc::now().to_rfc3339();
    session.exec_count += 1;
    save_session(&session)
}

/// Mark a session as stopped.
pub fn stop(name: &str) -> Result<()> {
    let mut session = get(name)?.ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;
    session.status = SessionStatus::Stopped;
    session.last_active = chrono::Utc::now().to_rfc3339();
    save_session(&session)
}

/// Mark a session as saved (with snapshot reference).
pub fn mark_saved(name: &str, snapshot_name: &str) -> Result<()> {
    let mut session = get(name)?.ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;
    session.status = SessionStatus::Saved;
    session.snapshot = Some(snapshot_name.to_string());
    session.last_active = chrono::Utc::now().to_rfc3339();
    save_session(&session)
}

/// Mark a session as running (for resume).
pub fn mark_running(name: &str) -> Result<()> {
    let mut session = get(name)?.ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;
    session.status = SessionStatus::Running;
    session.last_active = chrono::Utc::now().to_rfc3339();
    save_session(&session)
}

/// Delete a session.
pub fn delete(name: &str) -> Result<()> {
    let path = sessions_dir().join(format!("{}.json", name));
    if !path.exists() {
        bail!("Session '{}' not found", name);
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Persist session to disk.
fn save_session(session: &Session) -> Result<()> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", session.name));
    let content = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Format a duration between two RFC3339 timestamps as human-readable.
pub fn format_duration(from: &str, to: &str) -> String {
    let from_dt = chrono::DateTime::parse_from_rfc3339(from);
    let to_dt = chrono::DateTime::parse_from_rfc3339(to);

    if let (Ok(f), Ok(t)) = (from_dt, to_dt) {
        let dur = t.signed_duration_since(f);
        let secs = dur.num_seconds();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m", secs / 60)
        } else if secs < 86400 {
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            format!("{}h {}m", h, m)
        } else {
            format!("{}d", secs / 86400)
        }
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sessions_dir() {
        let dir = sessions_dir();
        assert!(dir.to_string_lossy().contains("agentkernel/sessions"));
    }

    #[test]
    fn test_list_empty() {
        let result = list();
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_missing() {
        let result = get("nonexistent-session-12345").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_missing() {
        let result = delete("nonexistent-session-12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_status_display() {
        assert_eq!(format!("{}", SessionStatus::Running), "running");
        assert_eq!(format!("{}", SessionStatus::Stopped), "stopped");
        assert_eq!(format!("{}", SessionStatus::Saved), "saved");
    }

    #[test]
    fn test_session_roundtrip() {
        let session = Session {
            name: "test-session".to_string(),
            sandbox: "session-test-session".to_string(),
            agent: "claude".to_string(),
            status: SessionStatus::Running,
            created_at: "2026-02-01T00:00:00Z".to_string(),
            last_active: "2026-02-01T01:00:00Z".to_string(),
            exec_count: 5,
            snapshot: None,
        };

        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-session");
        assert_eq!(parsed.exec_count, 5);
        assert_eq!(parsed.status, SessionStatus::Running);
    }

    #[test]
    fn test_session_with_snapshot() {
        let session = Session {
            name: "saved-session".to_string(),
            sandbox: "session-saved-session".to_string(),
            agent: "gemini".to_string(),
            status: SessionStatus::Saved,
            created_at: "2026-02-01T00:00:00Z".to_string(),
            last_active: "2026-02-01T02:00:00Z".to_string(),
            exec_count: 10,
            snapshot: Some("saved-session-20260201".to_string()),
        };

        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.snapshot, Some("saved-session-20260201".to_string()));
    }

    #[test]
    fn test_format_duration_seconds() {
        let dur = format_duration("2026-02-01T00:00:00Z", "2026-02-01T00:00:30Z");
        assert_eq!(dur, "30s");
    }

    #[test]
    fn test_format_duration_minutes() {
        let dur = format_duration("2026-02-01T00:00:00Z", "2026-02-01T00:05:00Z");
        assert_eq!(dur, "5m");
    }

    #[test]
    fn test_format_duration_hours() {
        let dur = format_duration("2026-02-01T00:00:00Z", "2026-02-01T02:14:00Z");
        assert_eq!(dur, "2h 14m");
    }

    #[test]
    fn test_format_duration_days() {
        let dur = format_duration("2026-02-01T00:00:00Z", "2026-02-03T00:00:00Z");
        assert_eq!(dur, "2d");
    }

    #[test]
    fn test_format_duration_invalid() {
        let dur = format_duration("not-a-date", "also-not");
        assert_eq!(dur, "unknown");
    }

    #[test]
    fn test_session_create_and_cleanup() {
        // Test the serialize/deserialize path with tempdir
        let dir = tempfile::TempDir::new().unwrap();
        let session = Session {
            name: "temp-test".to_string(),
            sandbox: "session-temp-test".to_string(),
            agent: "codex".to_string(),
            status: SessionStatus::Running,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_active: chrono::Utc::now().to_rfc3339(),
            exec_count: 0,
            snapshot: None,
        };

        let path = dir.path().join("temp-test.json");
        let content = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(&path, &content).unwrap();

        let loaded: Session =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.name, "temp-test");
        assert_eq!(loaded.agent, "codex");
    }
}
