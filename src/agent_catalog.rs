//! Authoritative metadata for supported coding agents.
//!
//! The same checked-in catalog is used by image smoke tests, the CLI, HTTP API,
//! and desktop app. Keep install commands and tested versions out of UI code.

use serde::Deserialize;
use std::sync::OnceLock;

const CATALOG_JSON: &str = include_str!("../examples/agents/tested-versions.json");

#[derive(Debug, Deserialize)]
struct Catalog {
    agents: Vec<AgentCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub package: Option<String>,
    pub version: String,
    pub install_command: String,
    pub integration_target: Option<String>,
    pub executable: String,
    pub smoke_arg: String,
    pub expected_output: String,
}

pub fn agents() -> &'static [AgentCatalogEntry] {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    &CATALOG
        .get_or_init(|| {
            serde_json::from_str(CATALOG_JSON).expect("tested agent catalog must be valid")
        })
        .agents
}

pub fn find(id: &str) -> Option<&'static AgentCatalogEntry> {
    agents().iter().find(|entry| {
        entry.id == id || (entry.id == "claude-code" && matches!(id, "claude" | "claude-code"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_has_unique_complete_agents() {
        let entries = agents();
        assert_eq!(entries.len(), 9);
        let ids: HashSet<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids.len(), entries.len());
        for entry in entries {
            assert!(!entry.executable.is_empty());
            assert!(!entry.install_command.is_empty());
            assert!(entry.install_command.contains(&entry.version) || entry.package.is_none());
        }
    }
}
