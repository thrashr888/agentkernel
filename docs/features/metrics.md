
# Prometheus Metrics

agentkernel exposes a `/metrics` endpoint in Prometheus text exposition format. Scrape it with any Prometheus-compatible system.

## Quick Start

Start the API server:

```bash
agentkernel serve
```

Verify metrics are available:

```bash
curl http://localhost:18888/metrics
```

Add a scrape job to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: "agentkernel"
    scrape_interval: 10s
    static_configs:
      - targets: ["localhost:18888"]
```

Restart Prometheus, then open **http://localhost:9090** and query `agentkernel_build_info` to confirm the target is up.

## Available Metrics

### HTTP API

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `agentkernel_http_requests_total` | counter | method, path, status | Total API requests |
| `agentkernel_http_request_duration_seconds` | histogram | method, path | Request latency |

Path labels are normalized — sandbox names become `:name`, so `/sandboxes/my-box/exec` is recorded as `/sandboxes/:name/exec`. This prevents cardinality explosion.

### Sandbox Lifecycle

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `agentkernel_sandbox_lifecycle_total` | counter | action, backend | Lifecycle events (created, started, stopped, removed) |
| `agentkernel_sandbox_lifecycle_duration_seconds` | histogram | action, backend | Operation latency |
| `agentkernel_sandboxes_active` | gauge | — | Current number of known sandboxes |

### Command Execution

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `agentkernel_commands_total` | counter | backend | Commands executed |
| `agentkernel_command_duration_seconds` | histogram | backend | Execution latency |

### Build Info

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `agentkernel_build_info` | gauge | version | Always 1; version label tracks the running release |

## Example Queries

Request rate over the last 5 minutes:

```promql
rate(agentkernel_http_requests_total[5m])
```

p95 API latency:

```promql
histogram_quantile(0.95, rate(agentkernel_http_request_duration_seconds_bucket[5m]))
```

Sandbox creation rate by backend:

```promql
rate(agentkernel_sandbox_lifecycle_total{action="created"}[5m])
```

Active sandboxes:

```promql
agentkernel_sandboxes_active
```

Average command execution time:

```promql
rate(agentkernel_command_duration_seconds_sum[5m]) / rate(agentkernel_command_duration_seconds_count[5m])
```

## Authentication

The `/metrics` endpoint does not require authentication (same as `/health`). If you need to restrict access, use network-level controls or a reverse proxy.

## Kubernetes / Nomad

For production deployments, point your Prometheus at the agentkernel service endpoint:

```yaml
# Kubernetes ServiceMonitor
scrape_configs:
  - job_name: "agentkernel"
    kubernetes_sd_configs:
      - role: endpoints
    relabel_configs:
      - source_labels: [__meta_kubernetes_service_name]
        regex: agentkernel
        action: keep
```

For Nomad, use Consul service discovery or static targets pointing at the agentkernel allocation address.
