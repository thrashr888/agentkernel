use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::api_client::ApiClient;

/// A configured server endpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Persisted user settings for the desktop app.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Which server name is currently active.
    #[serde(default)]
    pub active_server: Option<String>,
    /// List of configured servers.
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    pub theme: String,
    pub poll_interval_ms: u64,

    // Legacy fields — kept for backward compatibility during migration.
    // New code should use `servers` + `active_server`.
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let url = std::env::var("AGENTKERNEL_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:18888".to_string());
        let key = std::env::var("AGENTKERNEL_API_KEY").ok();
        Self {
            active_server: Some("Local".to_string()),
            servers: vec![ServerEntry {
                name: "Local".to_string(),
                url: url.clone(),
                api_key: key.clone(),
            }],
            theme: "system".to_string(),
            poll_interval_ms: 3000,
            api_url: url,
            api_key: key,
        }
    }
}

impl Settings {
    /// Path to the persisted settings file.
    fn path() -> PathBuf {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agentkernel-desktop");
        dir.join("settings.json")
    }

    /// Load settings from disk, falling back to defaults.
    /// Migrates old single-server settings to the new multi-server format.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path();
        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let mut settings: Settings = serde_json::from_str(&data)?;
            settings.migrate();
            Ok(settings)
        } else {
            Ok(Self::default())
        }
    }

    /// If the settings were saved before multi-server support, migrate the
    /// single api_url/api_key into a "Local" server entry.
    fn migrate(&mut self) {
        if self.servers.is_empty() && !self.api_url.is_empty() {
            self.servers.push(ServerEntry {
                name: "Local".to_string(),
                url: self.api_url.clone(),
                api_key: self.api_key.clone(),
            });
            if self.active_server.is_none() {
                self.active_server = Some("Local".to_string());
            }
        }
    }

    /// Return the active server entry (if any).
    pub fn active(&self) -> Option<&ServerEntry> {
        let name = self.active_server.as_deref()?;
        self.servers.iter().find(|s| s.name == name)
    }

    /// Persist settings to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        fs::write(&path, data)?;
        Ok(())
    }
}

/// Shared application state managed by Tauri.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub client: Mutex<ApiClient>,
}

impl AppState {
    /// Build an `ApiClient` from the active server in settings.
    pub fn client_from_settings(settings: &Settings) -> ApiClient {
        if let Some(server) = settings.active() {
            ApiClient::new(&server.url, server.api_key.as_deref())
        } else if !settings.api_url.is_empty() {
            // Fallback to legacy fields
            ApiClient::new(&settings.api_url, settings.api_key.as_deref())
        } else {
            ApiClient::new("http://localhost:18888", None)
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        let settings = Settings::load().unwrap_or_default();
        let client = Self::client_from_settings(&settings);
        Self {
            settings: Mutex::new(settings),
            client: Mutex::new(client),
        }
    }
}
