//! Prometheus metrics for the agentkernel HTTP API and sandbox lifecycle.
//!
//! All metrics use a dedicated Registry (not the global default) to avoid
//! collisions with any dependency that might also use the prometheus crate.

use prometheus::{
    HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Registry, TextEncoder,
};
use std::sync::LazyLock;

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

// ---- HTTP request metrics ----

static HTTP_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!("agentkernel_http_requests_total", "Total HTTP API requests"),
        &["method", "path", "status"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static HTTP_REQUEST_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "agentkernel_http_request_duration_seconds",
            "HTTP request latency in seconds",
        )
        .buckets(vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ]),
        &["method", "path"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(h.clone()))
        .expect("metric can be registered");
    h
});

// ---- Sandbox lifecycle metrics ----

static SANDBOX_LIFECYCLE_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_sandbox_lifecycle_total",
            "Sandbox lifecycle events"
        ),
        &["action", "backend"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static SANDBOX_LIFECYCLE_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "agentkernel_sandbox_lifecycle_duration_seconds",
            "Sandbox lifecycle operation latency in seconds",
        )
        .buckets(vec![
            0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
        ]),
        &["action", "backend"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(h.clone()))
        .expect("metric can be registered");
    h
});

static SANDBOXES_ACTIVE: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_sandboxes_active",
        "Number of currently known sandboxes",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

// ---- Firecracker full-state checkpoint storage metrics ----

static FULL_STATE_CHECKPOINT_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_full_state_checkpoint_bytes",
        "Bytes currently used by published and staging full-state checkpoints",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

static FULL_STATE_PUBLISHED_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_full_state_checkpoint_published_bytes",
        "Bytes currently used by published full-state checkpoints",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

static FULL_STATE_STAGING_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_full_state_checkpoint_staging_bytes",
        "Bytes currently used by full-state checkpoint staging",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

static FULL_STATE_GLOBAL_QUOTA_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_full_state_checkpoint_global_quota_bytes",
        "Configured global full-state checkpoint byte quota",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

static FULL_STATE_TENANT_QUOTA_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    let g = IntGauge::new(
        "agentkernel_full_state_checkpoint_tenant_quota_bytes",
        "Configured per-tenant full-state checkpoint byte quota",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g
});

static FULL_STATE_QUOTA_DENIALS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_full_state_checkpoint_quota_denials_total",
            "Full-state checkpoint preflight quota denials"
        ),
        &["scope"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static FULL_STATE_GC_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_full_state_checkpoint_gc_total",
            "Full-state checkpoint garbage-collection actions"
        ),
        &["kind"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static FULL_STATE_GC_FREED_BYTES_TOTAL: LazyLock<prometheus::IntCounter> = LazyLock::new(|| {
    let c = prometheus::IntCounter::new(
        "agentkernel_full_state_checkpoint_gc_freed_bytes_total",
        "Bytes freed by full-state checkpoint garbage collection",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

// ---- Command execution metrics ----

static COMMANDS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!("agentkernel_commands_total", "Total commands executed"),
        &["backend"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static COMMAND_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "agentkernel_command_duration_seconds",
            "Command execution latency in seconds",
        )
        .buckets(vec![
            0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
        ]),
        &["backend"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(h.clone()))
        .expect("metric can be registered");
    h
});

// ---- LLM request metrics ----

static LLM_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_llm_requests_total",
            "Total LLM API requests intercepted"
        ),
        &["provider", "model"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static LLM_TOKENS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!("agentkernel_llm_tokens_total", "Total LLM tokens consumed"),
        &["provider", "direction"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static LLM_SPEND_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_llm_spend_requests_total",
            "Total intercepted LLM requests by provider and model"
        ),
        &["provider", "model"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

static LLM_SPEND_TOKENS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        prometheus::opts!(
            "agentkernel_llm_spend_tokens_total",
            "Total intercepted LLM tokens by provider, model, and direction"
        ),
        &["provider", "model", "direction"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(c.clone()))
        .expect("metric can be registered");
    c
});

// ---- Build info ----

static BUILD_INFO: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    let g = IntGaugeVec::new(
        prometheus::opts!("agentkernel_build_info", "Build metadata"),
        &["version"],
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(g.clone()))
        .expect("metric can be registered");
    g.with_label_values(&[env!("CARGO_PKG_VERSION")]).set(1);
    g
});

// ==== Public instrumentation API ====

/// Record an HTTP request (counter + histogram).
pub fn record_http_request(method: &str, path: &str, status: u16, duration_secs: f64) {
    let normalized = normalize_path(path);
    let status_str = status.to_string();
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, &normalized, &status_str])
        .inc();
    HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[method, &normalized])
        .observe(duration_secs);
}

/// Record a sandbox lifecycle operation (counter + histogram).
pub fn record_sandbox_lifecycle(action: &str, backend: &str, duration_secs: f64) {
    SANDBOX_LIFECYCLE_TOTAL
        .with_label_values(&[action, backend])
        .inc();
    SANDBOX_LIFECYCLE_DURATION_SECONDS
        .with_label_values(&[action, backend])
        .observe(duration_secs);
}

pub fn inc_active_sandboxes() {
    SANDBOXES_ACTIVE.inc();
}

pub fn dec_active_sandboxes() {
    SANDBOXES_ACTIVE.dec();
}

pub fn set_active_sandboxes(count: i64) {
    SANDBOXES_ACTIVE.set(count);
}

/// Refresh full-state storage gauges after a quota scan or GC pass.
pub fn set_full_state_storage(
    global_bytes: u64,
    published_bytes: u64,
    staging_bytes: u64,
    global_quota_bytes: u64,
    tenant_quota_bytes: u64,
) {
    FULL_STATE_CHECKPOINT_BYTES.set(saturating_metric_value(global_bytes));
    FULL_STATE_PUBLISHED_BYTES.set(saturating_metric_value(published_bytes));
    FULL_STATE_STAGING_BYTES.set(saturating_metric_value(staging_bytes));
    FULL_STATE_GLOBAL_QUOTA_BYTES.set(saturating_metric_value(global_quota_bytes));
    FULL_STATE_TENANT_QUOTA_BYTES.set(saturating_metric_value(tenant_quota_bytes));
}

pub fn record_full_state_quota_denial(scope: &str) {
    FULL_STATE_QUOTA_DENIALS_TOTAL
        .with_label_values(&[scope])
        .inc();
}

pub fn record_full_state_gc(removed_published: u64, removed_staging: u64, freed_bytes: u64) {
    if removed_published > 0 {
        FULL_STATE_GC_TOTAL
            .with_label_values(&["published"])
            .inc_by(removed_published);
    }
    if removed_staging > 0 {
        FULL_STATE_GC_TOTAL
            .with_label_values(&["staging"])
            .inc_by(removed_staging);
    }
    if freed_bytes > 0 {
        FULL_STATE_GC_FREED_BYTES_TOTAL.inc_by(freed_bytes);
    }
}

fn saturating_metric_value(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

/// Record an LLM API request (counter + token counters).
pub fn record_llm_request(provider: &str, model: &str, input_tokens: u64, output_tokens: u64) {
    LLM_REQUESTS_TOTAL
        .with_label_values(&[provider, model])
        .inc();
    if input_tokens > 0 {
        LLM_TOKENS_TOTAL
            .with_label_values(&[provider, "input"])
            .inc_by(input_tokens);
    }
    if output_tokens > 0 {
        LLM_TOKENS_TOTAL
            .with_label_values(&[provider, "output"])
            .inc_by(output_tokens);
    }
}

/// Record identity-aware usage without exposing request bodies or secrets.
pub fn record_llm_spend(event: &crate::llm_intercept::LlmEvent) {
    let labels = [
        metric_dimension(Some(&event.provider)),
        metric_dimension(event.model.as_deref()),
    ];
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    LLM_SPEND_REQUESTS_TOTAL
        .with_label_values(&label_refs)
        .inc();
    for (direction, value) in [
        ("input", event.input_tokens.unwrap_or(0)),
        ("output", event.output_tokens.unwrap_or(0)),
        (
            "total",
            event.total_tokens.unwrap_or_else(|| {
                event
                    .input_tokens
                    .unwrap_or(0)
                    .saturating_add(event.output_tokens.unwrap_or(0))
            }),
        ),
    ] {
        if value > 0 {
            let mut token_labels = labels.to_vec();
            token_labels.push(direction.to_string());
            let token_refs: Vec<&str> = token_labels.iter().map(String::as_str).collect();
            LLM_SPEND_TOKENS_TOTAL
                .with_label_values(&token_refs)
                .inc_by(value);
        }
    }
}

fn metric_dimension(value: Option<&str>) -> String {
    let value = value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("unknown");
    value.chars().take(128).collect()
}

/// Record a command execution (counter + histogram).
pub fn record_command(backend: &str, duration_secs: f64) {
    COMMANDS_TOTAL.with_label_values(&[backend]).inc();
    COMMAND_DURATION_SECONDS
        .with_label_values(&[backend])
        .observe(duration_secs);
}

/// Encode all registered metrics into Prometheus text exposition format.
pub fn gather() -> String {
    // Ensure build_info is initialized
    let _ = &*BUILD_INFO;
    let encoder = TextEncoder::new();
    let families = REGISTRY.gather();
    encoder.encode_to_string(&families).unwrap_or_default()
}

/// Normalize dynamic path segments to prevent label cardinality explosion.
///
/// Replaces sandbox/snapshot/secret names with `:name` so that metrics
/// group by route pattern, not by individual resource.
fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut result = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            let prev = segments[i - 1];
            if matches!(
                prev,
                "sandboxes" | "snapshots" | "secrets" | "detached" | "hooks" | "usage"
            ) {
                result.push(":name");
                continue;
            }
            if prev == "pages" {
                result.push(":page");
                continue;
            }
        }
        result.push(seg);
    }
    format!("/{}", result.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_static_paths() {
        assert_eq!(normalize_path("/health"), "/health");
        assert_eq!(normalize_path("/sandboxes"), "/sandboxes");
        assert_eq!(normalize_path("/run"), "/run");
        assert_eq!(normalize_path("/run/stream"), "/run/stream");
    }

    #[test]
    fn test_normalize_dynamic_paths() {
        assert_eq!(normalize_path("/sandboxes/my-box"), "/sandboxes/:name");
        assert_eq!(
            normalize_path("/sandboxes/my-box/exec"),
            "/sandboxes/:name/exec"
        );
        assert_eq!(normalize_path("/snapshots/snap-1"), "/snapshots/:name");
        assert_eq!(normalize_path("/secrets/my-key"), "/secrets/:name");
    }

    #[test]
    fn test_normalize_browser_paths() {
        assert_eq!(
            normalize_path("/sandboxes/x/browser/pages/p1/click"),
            "/sandboxes/:name/browser/pages/:page/click"
        );
    }

    #[test]
    fn test_normalize_detached_paths() {
        assert_eq!(
            normalize_path("/sandboxes/x/exec/detached/cmd-1"),
            "/sandboxes/:name/exec/detached/:name"
        );
    }

    #[test]
    fn test_gather_produces_output() {
        let output = gather();
        assert!(output.contains("agentkernel_build_info"));
    }

    #[test]
    fn test_record_http_request() {
        record_http_request("GET", "/sandboxes", 200, 0.05);
        let output = gather();
        assert!(output.contains("agentkernel_http_requests_total"));
    }
}
