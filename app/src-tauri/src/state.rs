use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::api_client::ApiClient;

/// Persisted user settings for the desktop app.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub api_url: String,
    pub api_key: Option<String>,
    pub theme: String,
    pub poll_interval_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_url: std::env::var("AGENTKERNEL_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:18888".to_string()),
            api_key: std::env::var("AGENTKERNEL_API_KEY").ok(),
            theme: "system".to_string(),
            poll_interval_ms: 3000,
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
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path();
        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let settings: Settings = serde_json::from_str(&data)?;
            Ok(settings)
        } else {
            Ok(Self::default())
        }
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

impl Default for AppState {
    fn default() -> Self {
        let settings = Settings::load().unwrap_or_default();
        let client = ApiClient::new(&settings.api_url, settings.api_key.as_deref());
        Self {
            settings: Mutex::new(settings),
            client: Mutex::new(client),
        }
    }
}
