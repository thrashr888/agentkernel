//! OpenTelemetry integration for agentkernel.
//!
//! Initializes an OTLP/HTTP trace exporter and provides helpers for
//! extracting W3C `traceparent` context from incoming HTTP requests.
//! Each HTTP request handled by `http_api` is wrapped in a single
//! server span; sandbox-level sub-spans are not created.

use anyhow::Result;
use hyper::Request;
use hyper::body::Incoming;
use opentelemetry::trace::{SpanKind, TraceContextExt, Tracer, TracerProvider};
use opentelemetry::{Context, KeyValue};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Initialize an OTel tracer provider that exports spans via OTLP/HTTP.
pub fn init_tracer(endpoint: &str) -> Result<SdkTracerProvider> {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create OTLP span exporter: {}", e))?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name("agentkernel").build())
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}

/// Extract W3C `traceparent` from HTTP request headers into an OTel Context.
///
/// Returns the current (empty) context if no traceparent is present.
pub fn extract_context(req: &Request<Incoming>) -> Context {
    let traceparent = req
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok());

    let tracestate = req
        .headers()
        .get("tracestate")
        .and_then(|v| v.to_str().ok());

    match traceparent {
        Some(tp) => parse_traceparent(tp, tracestate),
        None => Context::current(),
    }
}

/// Extract the `traceparent` and `tracestate` header values as strings,
/// suitable for injecting into sandbox environment variables.
pub fn extract_trace_headers(req: &Request<Incoming>) -> (Option<String>, Option<String>) {
    let traceparent = req
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let tracestate = req
        .headers()
        .get("tracestate")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    (traceparent, tracestate)
}

/// Parse a W3C traceparent header into an OTel Context with a remote SpanContext.
fn parse_traceparent(traceparent: &str, tracestate: Option<&str>) -> Context {
    // traceparent format: version-trace_id-span_id-trace_flags
    // e.g. "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    let parts: Vec<&str> = traceparent.split('-').collect();
    if parts.len() != 4 {
        return Context::current();
    }

    let trace_id = match opentelemetry::trace::TraceId::from_hex(parts[1]) {
        Ok(id) => id,
        Err(_) => return Context::current(),
    };
    let span_id = match opentelemetry::trace::SpanId::from_hex(parts[2]) {
        Ok(id) => id,
        Err(_) => return Context::current(),
    };
    let trace_flags = u8::from_str_radix(parts[3], 16).unwrap_or(0);

    let mut state = opentelemetry::trace::TraceState::default();
    if let Some(ts) = tracestate
        && let Ok(parsed) = opentelemetry::trace::TraceState::from_key_value(
            ts.split(',')
                .filter_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    match (kv.next(), kv.next()) {
                        (Some(k), Some(v)) => Some((k.trim(), v.trim())),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>(),
        )
    {
        state = parsed;
    }

    let span_context = opentelemetry::trace::SpanContext::new(
        trace_id,
        span_id,
        opentelemetry::trace::TraceFlags::new(trace_flags),
        true, // remote
        state,
    );

    Context::current().with_remote_span_context(span_context)
}

/// Create and start a span for a sandbox operation.
///
/// Returns the span (SDK type). The span is automatically ended when dropped,
/// or you can call `finish_span` to add final attributes and status.
pub fn start_span(
    provider: &SdkTracerProvider,
    parent_ctx: &Context,
    operation: &str,
    attributes: Vec<KeyValue>,
) -> opentelemetry_sdk::trace::Span {
    let tracer = provider.tracer("agentkernel");
    tracer.build_with_context(
        tracer
            .span_builder(operation.to_string())
            .with_kind(SpanKind::Server)
            .with_attributes(attributes),
        parent_ctx,
    )
}

/// Record span completion with status and attributes, then end it.
pub fn finish_span(
    span: &mut opentelemetry_sdk::trace::Span,
    success: bool,
    attributes: Vec<KeyValue>,
) {
    use opentelemetry::trace::Span;
    for attr in attributes {
        span.set_attribute(attr);
    }
    if success {
        span.set_status(opentelemetry::trace::Status::Ok);
    } else {
        span.set_status(opentelemetry::trace::Status::error("operation failed"));
    }
    span.end();
}

/// Gracefully shut down the tracer provider, flushing pending spans.
#[allow(dead_code)]
pub fn shutdown(provider: &SdkTracerProvider) {
    if let Err(e) = provider.shutdown() {
        eprintln!("[otel] Shutdown error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_traceparent_valid() {
        let ctx = parse_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            None,
        );
        let span_ctx = ctx.span().span_context().clone();
        assert!(span_ctx.is_remote());
        assert_eq!(
            span_ctx.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(span_ctx.span_id().to_string(), "00f067aa0ba902b7");
    }

    #[test]
    fn test_parse_traceparent_invalid() {
        let ctx = parse_traceparent("garbage", None);
        // Should fallback to current context (no remote span)
        assert!(!ctx.span().span_context().is_remote());
    }

    #[test]
    fn test_parse_traceparent_with_tracestate() {
        let ctx = parse_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            Some("vendor1=value1,vendor2=value2"),
        );
        let span_ctx = ctx.span().span_context().clone();
        assert!(span_ctx.is_remote());
    }
}
