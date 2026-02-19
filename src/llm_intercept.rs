//! LLM traffic interception for the Gondolin proxy.
//!
//! Detects requests to known LLM API endpoints, extracts request metadata
//! (model, streaming flag) and response metadata (token counts), and
//! accumulates per-sandbox usage statistics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::RwLock;

// ---- LLM Domain Registry ----

/// How to extract token counts from a provider's response JSON.
#[derive(Debug, Clone, Copy)]
pub enum TokenFormat {
    /// `response.usage.{prompt_tokens, completion_tokens, total_tokens}`
    OpenAi,
    /// `response.usage.{input_tokens, output_tokens}`
    Anthropic,
    /// `response.usageMetadata.{promptTokenCount, candidatesTokenCount, totalTokenCount}`
    Google,
    /// `response.meta.tokens.{input_tokens, output_tokens}`
    Cohere,
}

/// A known LLM API provider.
#[derive(Debug, Clone)]
pub struct LlmProvider {
    pub name: &'static str,
    pub token_format: TokenFormat,
}

/// Registry of known LLM API domains.
pub struct LlmDomainRegistry {
    domains: HashMap<String, LlmProvider>,
}

impl LlmDomainRegistry {
    /// Built-in registry with all major LLM providers.
    pub fn default_registry() -> Self {
        let mut domains = HashMap::new();
        let providers: &[(&str, &str, TokenFormat)] = &[
            ("api.openai.com", "openai", TokenFormat::OpenAi),
            ("api.anthropic.com", "anthropic", TokenFormat::Anthropic),
            (
                "generativelanguage.googleapis.com",
                "google",
                TokenFormat::Google,
            ),
            ("api.deepseek.com", "deepseek", TokenFormat::OpenAi),
            ("api.groq.com", "groq", TokenFormat::OpenAi),
            ("api.mistral.ai", "mistral", TokenFormat::OpenAi),
            ("api.cohere.com", "cohere", TokenFormat::Cohere),
            ("api.together.xyz", "together", TokenFormat::OpenAi),
            ("api.fireworks.ai", "fireworks", TokenFormat::OpenAi),
        ];
        for &(domain, name, format) in providers {
            domains.insert(
                domain.to_string(),
                LlmProvider {
                    name,
                    token_format: format,
                },
            );
        }
        Self { domains }
    }

    /// Empty registry (LLM interception disabled).
    pub fn empty() -> Self {
        Self {
            domains: HashMap::new(),
        }
    }

    /// Add custom LLM domains (assumed OpenAI-compatible token format).
    pub fn with_custom_domains(mut self, extras: &[String]) -> Self {
        for domain in extras {
            self.domains.insert(
                domain.clone(),
                LlmProvider {
                    name: "custom",
                    token_format: TokenFormat::OpenAi,
                },
            );
        }
        self
    }

    /// Check if a host is a known LLM API. Strips port if present.
    pub fn lookup(&self, host: &str) -> Option<&LlmProvider> {
        let host_only = host.split(':').next().unwrap_or(host);
        self.domains.get(host_only)
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

// ---- LLM Event ----

/// An intercepted LLM API request/response pair.
#[derive(Debug, Clone, Serialize)]
pub struct LlmEvent {
    pub timestamp: String,
    pub sandbox: String,
    pub provider: String,
    pub host: String,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub streaming: bool,
    pub secret_injected: bool,
    /// Source of the API key: "org", "sandbox", or "none"
    pub key_source: String,
}

// ---- Body Parsing ----

/// Extract the `model` field from a request body (best-effort).
pub fn extract_model_from_request(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Partial {
        model: Option<String>,
    }
    let cap = body.len().min(8192);
    serde_json::from_slice::<Partial>(&body[..cap]).ok()?.model
}

/// Check if the request body contains `"stream": true`.
pub fn extract_streaming_from_request(body: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Partial {
        stream: Option<bool>,
    }
    let cap = body.len().min(8192);
    serde_json::from_slice::<Partial>(&body[..cap])
        .ok()
        .and_then(|p| p.stream)
        .unwrap_or(false)
}

/// Extracted token usage from an LLM response.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

/// Extract token usage from a response body based on the provider's format.
pub fn extract_token_usage(body: &[u8], format: &TokenFormat) -> TokenUsage {
    let val: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return TokenUsage::default(),
    };

    match format {
        TokenFormat::OpenAi => {
            let usage = &val["usage"];
            TokenUsage {
                input_tokens: usage["prompt_tokens"].as_u64(),
                output_tokens: usage["completion_tokens"].as_u64(),
                total_tokens: usage["total_tokens"].as_u64(),
            }
        }
        TokenFormat::Anthropic => {
            let usage = &val["usage"];
            let input = usage["input_tokens"].as_u64();
            let output = usage["output_tokens"].as_u64();
            TokenUsage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: match (input, output) {
                    (Some(i), Some(o)) => Some(i + o),
                    _ => None,
                },
            }
        }
        TokenFormat::Google => {
            let meta = &val["usageMetadata"];
            TokenUsage {
                input_tokens: meta["promptTokenCount"].as_u64(),
                output_tokens: meta["candidatesTokenCount"].as_u64(),
                total_tokens: meta["totalTokenCount"].as_u64(),
            }
        }
        TokenFormat::Cohere => {
            let tokens = &val["meta"]["tokens"];
            let input = tokens["input_tokens"].as_u64();
            let output = tokens["output_tokens"].as_u64();
            TokenUsage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: match (input, output) {
                    (Some(i), Some(o)) => Some(i + o),
                    _ => None,
                },
            }
        }
    }
}

// ---- In-Memory Usage Store ----

/// Global LLM usage accumulator.
pub static LLM_USAGE: LazyLock<RwLock<LlmUsageStore>> =
    LazyLock::new(|| RwLock::new(LlmUsageStore::default()));

/// Accumulated LLM usage across all sandboxes.
#[derive(Debug, Default, Clone, Serialize)]
pub struct LlmUsageStore {
    pub by_sandbox: HashMap<String, Vec<LlmUsageEntry>>,
}

/// Accumulated usage for a single provider+model combination within a sandbox.
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsageEntry {
    pub provider: String,
    pub model: String,
    pub request_count: u64,
    pub streaming_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub last_request: String,
}

impl LlmUsageStore {
    /// Record an LLM event, upserting by sandbox + provider + model.
    pub fn record(&mut self, event: &LlmEvent) {
        let model = event.model.clone().unwrap_or_else(|| "unknown".to_string());
        let entries = self.by_sandbox.entry(event.sandbox.clone()).or_default();

        if let Some(entry) = entries
            .iter_mut()
            .find(|e| e.provider == event.provider && e.model == model)
        {
            entry.request_count += 1;
            if event.streaming {
                entry.streaming_count += 1;
            }
            entry.total_input_tokens += event.input_tokens.unwrap_or(0);
            entry.total_output_tokens += event.output_tokens.unwrap_or(0);
            entry.total_tokens += event.total_tokens.unwrap_or(0);
            entry.last_request.clone_from(&event.timestamp);
        } else {
            entries.push(LlmUsageEntry {
                provider: event.provider.clone(),
                model,
                request_count: 1,
                streaming_count: if event.streaming { 1 } else { 0 },
                total_input_tokens: event.input_tokens.unwrap_or(0),
                total_output_tokens: event.output_tokens.unwrap_or(0),
                total_tokens: event.total_tokens.unwrap_or(0),
                last_request: event.timestamp.clone(),
            });
        }
    }

    pub fn usage_for_sandbox(&self, sandbox: &str) -> Vec<LlmUsageEntry> {
        self.by_sandbox.get(sandbox).cloned().unwrap_or_default()
    }

    pub fn all_usage(&self) -> &HashMap<String, Vec<LlmUsageEntry>> {
        &self.by_sandbox
    }

    pub fn clear_sandbox(&mut self, sandbox: &str) {
        self.by_sandbox.remove(sandbox);
    }
}

/// Record an LLM event to the global usage store.
///
/// Callers should also invoke `crate::metrics::record_llm_request()` for
/// Prometheus counters when the metrics module is available (binary crate).
pub async fn record_llm_event(event: &LlmEvent) {
    LLM_USAGE.write().await.record(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_has_known_providers() {
        let reg = LlmDomainRegistry::default_registry();
        assert!(reg.lookup("api.openai.com").is_some());
        assert!(reg.lookup("api.anthropic.com").is_some());
        assert!(reg.lookup("generativelanguage.googleapis.com").is_some());
        assert!(reg.lookup("api.deepseek.com").is_some());
        assert!(reg.lookup("api.groq.com").is_some());
        assert!(reg.lookup("api.mistral.ai").is_some());
        assert!(reg.lookup("api.cohere.com").is_some());
        assert!(reg.lookup("example.com").is_none());
    }

    #[test]
    fn test_lookup_strips_port() {
        let reg = LlmDomainRegistry::default_registry();
        assert!(reg.lookup("api.openai.com:443").is_some());
    }

    #[test]
    fn test_custom_domains() {
        let reg = LlmDomainRegistry::default_registry()
            .with_custom_domains(&["my-llm.internal.com".to_string()]);
        assert!(reg.lookup("my-llm.internal.com").is_some());
        assert_eq!(reg.lookup("my-llm.internal.com").unwrap().name, "custom");
    }

    #[test]
    fn test_empty_registry() {
        let reg = LlmDomainRegistry::empty();
        assert!(reg.is_empty());
        assert!(reg.lookup("api.openai.com").is_none());
    }

    #[test]
    fn test_extract_model_openai() {
        let body = br#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#;
        assert_eq!(extract_model_from_request(body), Some("gpt-4".to_string()));
    }

    #[test]
    fn test_extract_model_anthropic() {
        let body = br#"{"model":"claude-3-opus-20240229","max_tokens":1024}"#;
        assert_eq!(
            extract_model_from_request(body),
            Some("claude-3-opus-20240229".to_string())
        );
    }

    #[test]
    fn test_extract_model_missing() {
        let body = br#"{"prompt":"hello"}"#;
        assert_eq!(extract_model_from_request(body), None);
    }

    #[test]
    fn test_extract_model_invalid_json() {
        assert_eq!(extract_model_from_request(b"not json"), None);
    }

    #[test]
    fn test_extract_streaming_true() {
        let body = br#"{"model":"gpt-4","stream":true}"#;
        assert!(extract_streaming_from_request(body));
    }

    #[test]
    fn test_extract_streaming_false() {
        let body = br#"{"model":"gpt-4","stream":false}"#;
        assert!(!extract_streaming_from_request(body));
    }

    #[test]
    fn test_extract_streaming_missing() {
        let body = br#"{"model":"gpt-4"}"#;
        assert!(!extract_streaming_from_request(body));
    }

    #[test]
    fn test_extract_tokens_openai() {
        let body = br#"{"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#;
        let usage = extract_token_usage(body, &TokenFormat::OpenAi);
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
    }

    #[test]
    fn test_extract_tokens_anthropic() {
        let body = br#"{"usage":{"input_tokens":5,"output_tokens":15}}"#;
        let usage = extract_token_usage(body, &TokenFormat::Anthropic);
        assert_eq!(usage.input_tokens, Some(5));
        assert_eq!(usage.output_tokens, Some(15));
        assert_eq!(usage.total_tokens, Some(20));
    }

    #[test]
    fn test_extract_tokens_google() {
        let body = br#"{"usageMetadata":{"promptTokenCount":8,"candidatesTokenCount":12,"totalTokenCount":20}}"#;
        let usage = extract_token_usage(body, &TokenFormat::Google);
        assert_eq!(usage.input_tokens, Some(8));
        assert_eq!(usage.output_tokens, Some(12));
        assert_eq!(usage.total_tokens, Some(20));
    }

    #[test]
    fn test_extract_tokens_cohere() {
        let body = br#"{"meta":{"tokens":{"input_tokens":3,"output_tokens":7}}}"#;
        let usage = extract_token_usage(body, &TokenFormat::Cohere);
        assert_eq!(usage.input_tokens, Some(3));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.total_tokens, Some(10));
    }

    #[test]
    fn test_extract_tokens_invalid_json() {
        let usage = extract_token_usage(b"not json", &TokenFormat::OpenAi);
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.total_tokens, None);
    }

    #[test]
    fn test_extract_tokens_missing_usage() {
        let body = br#"{"id":"chatcmpl-123","choices":[]}"#;
        let usage = extract_token_usage(body, &TokenFormat::OpenAi);
        assert_eq!(usage.input_tokens, None);
    }

    #[test]
    fn test_usage_store_record_and_query() {
        let mut store = LlmUsageStore::default();
        let event = LlmEvent {
            timestamp: "2026-01-01T00:00:00Z".into(),
            sandbox: "test-sb".into(),
            provider: "openai".into(),
            host: "api.openai.com".into(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            model: Some("gpt-4".into()),
            status: Some(200),
            latency_ms: Some(500),
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            streaming: false,
            secret_injected: true,
            key_source: "sandbox".into(),
        };
        store.record(&event);
        store.record(&event);

        let entries = store.usage_for_sandbox("test-sb");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].request_count, 2);
        assert_eq!(entries[0].total_input_tokens, 20);
        assert_eq!(entries[0].total_output_tokens, 40);
        assert_eq!(entries[0].total_tokens, 60);
        assert_eq!(entries[0].streaming_count, 0);
    }

    #[test]
    fn test_usage_store_streaming_count() {
        let mut store = LlmUsageStore::default();
        let event = LlmEvent {
            timestamp: "2026-01-01T00:00:00Z".into(),
            sandbox: "test-sb".into(),
            provider: "openai".into(),
            host: "api.openai.com".into(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            model: Some("gpt-4".into()),
            status: Some(200),
            latency_ms: Some(500),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            streaming: true,
            secret_injected: true,
            key_source: "sandbox".into(),
        };
        store.record(&event);
        let entries = store.usage_for_sandbox("test-sb");
        assert_eq!(entries[0].streaming_count, 1);
    }

    #[test]
    fn test_usage_store_multiple_models() {
        let mut store = LlmUsageStore::default();
        let base = LlmEvent {
            timestamp: "2026-01-01T00:00:00Z".into(),
            sandbox: "test-sb".into(),
            provider: "openai".into(),
            host: "api.openai.com".into(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            model: Some("gpt-4".into()),
            status: Some(200),
            latency_ms: Some(500),
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            streaming: false,
            secret_injected: true,
            key_source: "sandbox".into(),
        };
        store.record(&base);

        let mut turbo = base.clone();
        turbo.model = Some("gpt-4-turbo".into());
        store.record(&turbo);

        let entries = store.usage_for_sandbox("test-sb");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_usage_store_clear() {
        let mut store = LlmUsageStore::default();
        let event = LlmEvent {
            timestamp: "2026-01-01T00:00:00Z".into(),
            sandbox: "test-sb".into(),
            provider: "openai".into(),
            host: "api.openai.com".into(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            model: Some("gpt-4".into()),
            status: Some(200),
            latency_ms: Some(500),
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            streaming: false,
            secret_injected: true,
            key_source: "sandbox".into(),
        };
        store.record(&event);
        store.clear_sandbox("test-sb");
        assert!(store.usage_for_sandbox("test-sb").is_empty());
    }

    #[test]
    fn test_usage_store_unknown_sandbox() {
        let store = LlmUsageStore::default();
        assert!(store.usage_for_sandbox("nonexistent").is_empty());
    }
}
