//! HTTP client for fetching policy bundles from the enterprise policy server.
//!
//! Supports authenticated requests via Bearer token or API key header,
//! and background polling for policy updates.

#![cfg(feature = "enterprise")]

use anyhow::{Result, Context as _};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

use super::signing::PolicyBundle;

/// HTTP client for the enterprise policy server.
pub struct PolicyClient {
    /// Base URL of the policy server (e.g., "https://policy.acme-corp.com")
    server_url: String,
    /// API key for authentication
    api_key: Option<String>,
    /// HTTP client
    http_client: reqwest::Client,
}

impl PolicyClient {
    /// Create a new PolicyClient.
    ///
    /// # Arguments
    /// * `server_url` - Base URL of the policy server
    /// * `api_key` - Optional API key for authentication
    pub fn new(server_url: &str, api_key: Option<String>) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(format!("agentkernel/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            api_key,
            http_client,
        })
    }

    /// Fetch the current policy bundle from the server.
    ///
    /// Makes a GET request to `/v1/policies` and deserializes the response
    /// as a PolicyBundle.
    pub async fn fetch_bundle(&self) -> Result<PolicyBundle> {
        let url = format!("{}/v1/policies", self.server_url);

        let mut request = self.http_client.get(&url);

        // Add authentication
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request
            .send()
            .await
            .context("Failed to connect to policy server")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            anyhow::bail!(
                "Policy server returned {}: {}",
                status,
                body
            );
        }

        let bundle: PolicyBundle = response
            .json()
            .await
            .context("Failed to parse policy bundle response")?;

        Ok(bundle)
    }

    /// Start background polling for policy updates.
    ///
    /// Fetches the policy bundle at the given interval and sends updates
    /// through the returned watch channel. Stops when the shutdown signal
    /// is received.
    ///
    /// # Arguments
    /// * `interval` - How often to poll for updates
    /// * `shutdown` - Receiver that signals when to stop polling
    ///
    /// # Returns
    /// A watch receiver that receives new PolicyBundle values on updates.
    pub fn poll(
        self: Arc<Self>,
        interval: Duration,
        mut shutdown: watch::Receiver<bool>,
    ) -> watch::Receiver<Option<PolicyBundle>> {
        let (tx, rx) = watch::channel(None);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // First tick is immediate
            ticker.tick().await;

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        match self.fetch_bundle().await {
                            Ok(bundle) => {
                                let _ = tx.send(Some(bundle));
                            }
                            Err(e) => {
                                eprintln!("[enterprise] Failed to fetch policy bundle: {}", e);
                            }
                        }
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = PolicyClient::new("https://policy.example.com", None);
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_creation_with_key() {
        let client = PolicyClient::new(
            "https://policy.example.com",
            Some("test-api-key".to_string()),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn test_server_url_normalization() {
        let client =
            PolicyClient::new("https://policy.example.com/", None).unwrap();
        assert_eq!(client.server_url, "https://policy.example.com");
    }
}
