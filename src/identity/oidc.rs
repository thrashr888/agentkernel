//! OIDC Device Authorization Flow for CLI authentication.
//!
//! Implements the OAuth 2.0 Device Authorization Grant (RFC 8628) for
//! authenticating CLI users via browser-based OIDC providers (Okta, Azure AD,
//! Google Workspace, Auth0, etc.).
//!
//! Flow:
//! 1. CLI requests device code from authorization server
//! 2. User visits verification URL and enters the code
//! 3. CLI polls token endpoint until user completes auth
//! 4. Tokens are stored locally at ~/.agentkernel/auth/tokens.json

#[cfg(feature = "enterprise")]
use anyhow::{Context, Result, bail};
#[cfg(feature = "enterprise")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "enterprise")]
use std::path::PathBuf;

/// OpenID Connect Discovery configuration.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Deserialize)]
pub struct OidcConfig {
    /// Authorization endpoint URL
    pub authorization_endpoint: String,
    /// Token endpoint URL
    pub token_endpoint: String,
    /// Device authorization endpoint URL (RFC 8628)
    #[serde(default)]
    pub device_authorization_endpoint: Option<String>,
    /// JWKS (JSON Web Key Set) URI for token verification
    pub jwks_uri: String,
    /// Issuer identifier
    pub issuer: String,
    /// UserInfo endpoint
    #[serde(default)]
    pub userinfo_endpoint: Option<String>,
    /// Supported response types
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    /// Supported grant types
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Response from the device authorization endpoint.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthResponse {
    /// Device verification code (used by client for polling)
    pub device_code: String,
    /// User code to display (user enters this in browser)
    pub user_code: String,
    /// Verification URI where user enters the code
    pub verification_uri: String,
    /// Optional: complete verification URI with code pre-filled
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Lifetime of device_code and user_code in seconds
    pub expires_in: u64,
    /// Polling interval in seconds (default: 5)
    #[serde(default = "default_interval")]
    pub interval: u64,
}

#[cfg(feature = "enterprise")]
fn default_interval() -> u64 {
    5
}

/// Token response from the token endpoint.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// OAuth 2.0 access token
    pub access_token: String,
    /// OIDC ID token (JWT containing user claims)
    #[serde(default)]
    pub id_token: Option<String>,
    /// Refresh token for obtaining new access tokens
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token type (usually "Bearer")
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Access token lifetime in seconds
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// Granted scopes (space-separated)
    #[serde(default)]
    pub scope: Option<String>,
}

#[cfg(feature = "enterprise")]
fn default_token_type() -> String {
    "Bearer".to_string()
}

/// Error response from the token endpoint during polling.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Deserialize)]
pub struct TokenErrorResponse {
    /// Error code
    pub error: String,
    /// Human-readable error description
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Stored token data persisted to disk.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    /// Access token
    pub access_token: String,
    /// ID token
    pub id_token: Option<String>,
    /// Refresh token
    pub refresh_token: Option<String>,
    /// When the access token expires (ISO 8601)
    pub expires_at: Option<String>,
    /// OIDC issuer URL
    pub issuer: String,
    /// Client ID used for authentication
    pub client_id: String,
}

/// OIDC Device Authorization Flow handler.
#[cfg(feature = "enterprise")]
pub struct OidcDeviceFlow {
    /// OIDC discovery URL (issuer URL)
    pub discovery_url: String,
    /// OAuth client ID
    pub client_id: String,
    /// Scopes to request
    pub scopes: Vec<String>,
    /// HTTP client for making requests
    client: reqwest::Client,
}

#[cfg(feature = "enterprise")]
impl OidcDeviceFlow {
    /// Create a new OIDC device flow handler.
    pub fn new(discovery_url: String, client_id: String) -> Self {
        Self {
            discovery_url,
            client_id,
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            client: reqwest::Client::new(),
        }
    }

    /// Set custom scopes for the flow.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Discover OIDC configuration from the issuer's well-known endpoint.
    pub async fn discover(&self) -> Result<OidcConfig> {
        let well_known_url = format!(
            "{}/.well-known/openid-configuration",
            self.discovery_url.trim_end_matches('/')
        );

        let config: OidcConfig = self
            .client
            .get(&well_known_url)
            .send()
            .await
            .context("Failed to fetch OIDC discovery document")?
            .json()
            .await
            .context("Failed to parse OIDC discovery document")?;

        Ok(config)
    }

    /// Start the device authorization flow.
    ///
    /// Returns a DeviceAuthResponse containing the user code and verification URL.
    /// The user should be directed to visit the verification URL and enter the code.
    pub async fn start_device_flow(&self) -> Result<DeviceAuthResponse> {
        let config = self.discover().await?;

        let device_endpoint = config
            .device_authorization_endpoint
            .as_ref()
            .context("OIDC provider does not support device authorization flow")?;

        let scope = self.scopes.join(" ");

        let response = self
            .client
            .post(device_endpoint)
            .form(&[("client_id", &self.client_id), ("scope", &scope)])
            .send()
            .await
            .context("Failed to request device authorization")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Device authorization request failed ({}): {}", status, body);
        }

        let device_auth: DeviceAuthResponse = response
            .json()
            .await
            .context("Failed to parse device authorization response")?;

        Ok(device_auth)
    }

    /// Poll the token endpoint for a completed device authorization.
    ///
    /// This will block until the user completes authentication, the device code
    /// expires, or an unrecoverable error occurs.
    pub async fn poll_for_token(&self, device_auth: &DeviceAuthResponse) -> Result<TokenResponse> {
        let config = self.discover().await?;
        let token_endpoint = &config.token_endpoint;
        let interval = std::time::Duration::from_secs(device_auth.interval);
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(device_auth.expires_in);

        loop {
            // Respect the polling interval
            tokio::time::sleep(interval).await;

            // Check if the device code has expired
            if std::time::Instant::now() >= deadline {
                bail!("Device authorization expired. Please try again.");
            }

            let response = self
                .client
                .post(token_endpoint)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &device_auth.device_code),
                    ("client_id", &self.client_id),
                ])
                .send()
                .await
                .context("Failed to poll token endpoint")?;

            if response.status().is_success() {
                let token: TokenResponse = response
                    .json()
                    .await
                    .context("Failed to parse token response")?;
                return Ok(token);
            }

            // Parse error response
            let error_body = response.text().await.unwrap_or_default();
            let error: TokenErrorResponse =
                serde_json::from_str(&error_body).unwrap_or(TokenErrorResponse {
                    error: "unknown".to_string(),
                    error_description: Some(error_body),
                });

            match error.error.as_str() {
                "authorization_pending" => {
                    // User hasn't completed auth yet, continue polling
                    continue;
                }
                "slow_down" => {
                    // Server wants us to slow down, add 5 seconds
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                "expired_token" => {
                    bail!("Device authorization expired. Please try again.");
                }
                "access_denied" => {
                    bail!("User denied authorization.");
                }
                other => {
                    bail!(
                        "Token request failed: {} ({})",
                        other,
                        error
                            .error_description
                            .unwrap_or_else(|| "no description".to_string())
                    );
                }
            }
        }
    }

    /// Get the path to the token storage file.
    pub fn token_store_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".agentkernel")
            .join("auth")
            .join("tokens.json")
    }

    /// Store tokens securely to disk.
    ///
    /// Tokens are stored at ~/.agentkernel/auth/tokens.json with permissions 0600.
    pub fn store_tokens(&self, token: &TokenResponse) -> Result<()> {
        let path = Self::token_store_path();

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create auth directory")?;
        }

        // Calculate expiration time
        let expires_at = token.expires_in.map(|secs| {
            let expiry = chrono::Utc::now() + chrono::Duration::seconds(secs as i64);
            expiry.to_rfc3339()
        });

        let stored = StoredTokens {
            access_token: token.access_token.clone(),
            id_token: token.id_token.clone(),
            refresh_token: token.refresh_token.clone(),
            expires_at,
            issuer: self.discovery_url.clone(),
            client_id: self.client_id.clone(),
        };

        let content =
            serde_json::to_string_pretty(&stored).context("Failed to serialize tokens")?;

        std::fs::write(&path, &content).context("Failed to write token file")?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&path, permissions)
                .context("Failed to set token file permissions")?;
        }

        Ok(())
    }

    /// Load stored tokens from disk.
    pub fn load_tokens() -> Result<Option<StoredTokens>> {
        let path = Self::token_store_path();

        if !path.exists() {
            return Ok(None);
        }

        // Verify file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata =
                std::fs::metadata(&path).context("Failed to read token file metadata")?;
            let mode = metadata.permissions().mode() & 0o777;
            if mode != 0o600 {
                bail!(
                    "Token file has insecure permissions {:o} (expected 0600). \
                     Fix with: chmod 600 {}",
                    mode,
                    path.display()
                );
            }
        }

        let content = std::fs::read_to_string(&path).context("Failed to read token file")?;

        let tokens: StoredTokens =
            serde_json::from_str(&content).context("Failed to parse stored tokens")?;

        // Check if access token is expired
        if let Some(ref expires_at) = tokens.expires_at
            && let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at)
            && expiry < chrono::Utc::now()
        {
            // Token expired, but might have refresh token
            if tokens.refresh_token.is_some() {
                // Return tokens so caller can attempt refresh
                return Ok(Some(tokens));
            }
            return Ok(None);
        }

        Ok(Some(tokens))
    }

    /// Clear stored tokens (logout).
    pub fn clear_tokens() -> Result<()> {
        let path = Self::token_store_path();
        if path.exists() {
            std::fs::remove_file(&path).context("Failed to remove token file")?;
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "enterprise"))]
mod tests {
    use super::*;

    #[test]
    fn test_device_auth_response_deserialization() {
        let json = r#"{
            "device_code": "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS",
            "user_code": "WDJB-MJHT",
            "verification_uri": "https://example.com/device",
            "verification_uri_complete": "https://example.com/device?user_code=WDJB-MJHT",
            "expires_in": 1800,
            "interval": 5
        }"#;

        let response: DeviceAuthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.user_code, "WDJB-MJHT");
        assert_eq!(response.expires_in, 1800);
        assert_eq!(response.interval, 5);
        assert!(response.verification_uri_complete.is_some());
    }

    #[test]
    fn test_device_auth_response_minimal() {
        let json = r#"{
            "device_code": "abc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://example.com/device",
            "expires_in": 600
        }"#;

        let response: DeviceAuthResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.interval, 5); // default
        assert!(response.verification_uri_complete.is_none());
    }

    #[test]
    fn test_token_response_deserialization() {
        let json = r#"{
            "access_token": "eyJhbGciOi...",
            "id_token": "eyJhbGciOi...",
            "refresh_token": "v1.MjQ1...",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "openid profile email"
        }"#;

        let token: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(token.token_type, "Bearer");
        assert_eq!(token.expires_in, Some(3600));
        assert!(token.id_token.is_some());
        assert!(token.refresh_token.is_some());
    }

    #[test]
    fn test_token_response_minimal() {
        let json = r#"{
            "access_token": "eyJhbGciOi..."
        }"#;

        let token: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(token.token_type, "Bearer"); // default
        assert!(token.id_token.is_none());
        assert!(token.refresh_token.is_none());
        assert!(token.expires_in.is_none());
    }

    #[test]
    fn test_token_error_response_deserialization() {
        let json = r#"{
            "error": "authorization_pending",
            "error_description": "The user has not yet completed authorization"
        }"#;

        let error: TokenErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(error.error, "authorization_pending");
        assert!(error.error_description.is_some());
    }

    #[test]
    fn test_oidc_config_deserialization() {
        let json = r#"{
            "issuer": "https://accounts.example.com",
            "authorization_endpoint": "https://accounts.example.com/authorize",
            "token_endpoint": "https://accounts.example.com/token",
            "device_authorization_endpoint": "https://accounts.example.com/device/code",
            "jwks_uri": "https://accounts.example.com/.well-known/jwks.json",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "urn:ietf:params:oauth:grant-type:device_code"]
        }"#;

        let config: OidcConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.issuer, "https://accounts.example.com");
        assert!(config.device_authorization_endpoint.is_some());
        assert!(
            config
                .grant_types_supported
                .contains(&"urn:ietf:params:oauth:grant-type:device_code".to_string())
        );
    }

    #[test]
    fn test_stored_tokens_serialization() {
        let stored = StoredTokens {
            access_token: "access-123".to_string(),
            id_token: Some("id-456".to_string()),
            refresh_token: Some("refresh-789".to_string()),
            expires_at: Some("2025-12-31T23:59:59+00:00".to_string()),
            issuer: "https://example.com".to_string(),
            client_id: "my-client".to_string(),
        };

        let json = serde_json::to_string(&stored).unwrap();
        let deserialized: StoredTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.access_token, "access-123");
        assert_eq!(deserialized.client_id, "my-client");
    }

    #[test]
    fn test_token_store_path() {
        let path = OidcDeviceFlow::token_store_path();
        assert!(path.to_string_lossy().contains("agentkernel"));
        assert!(path.to_string_lossy().contains("tokens.json"));
    }

    #[test]
    fn test_oidc_device_flow_new() {
        let flow = OidcDeviceFlow::new(
            "https://accounts.example.com".to_string(),
            "my-client-id".to_string(),
        );
        assert_eq!(flow.discovery_url, "https://accounts.example.com");
        assert_eq!(flow.client_id, "my-client-id");
        assert!(flow.scopes.contains(&"openid".to_string()));
    }

    #[test]
    fn test_oidc_device_flow_with_scopes() {
        let flow = OidcDeviceFlow::new("https://example.com".to_string(), "client".to_string())
            .with_scopes(vec!["openid".to_string(), "custom:read".to_string()]);

        assert_eq!(flow.scopes.len(), 2);
        assert!(flow.scopes.contains(&"custom:read".to_string()));
    }
}
