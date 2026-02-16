# Durable Protocol

Common schema and wire protocol for Durable Workflows and Durable Actors across all SDKs.

## Why A Shared Protocol?

Yes, a single protocol is required for:

- Cross-language interoperability (Node, Python, Go, Rust, Swift)
- Deterministic replay behavior across runtimes
- Stable storage format for event histories
- Consistent error handling, retries, and idempotency

Without this, each SDK drifts in payload shapes and replay semantics.

## Scope

The durable protocol defines:

- Canonical identifiers: `instance_id`, `object_id`, `activity_id`, `event_id`, `alarm_id` (UUIDv7)
- Canonical envelopes for orchestration events and object calls
- Canonical activity/object result envelope
- Error taxonomy and retryability semantics
- Versioning and compatibility rules

## Canonical Envelopes

### Orchestration Start Request

```json
{
  "protocol_version": "1.0",
  "name": "etl-pipeline",
  "instance_id": "019abc12-...",
  "input": {"url": "https://data.example.com"},
  "metadata": {"tenant_id": "acme"}
}
```

### Orchestration Event

```json
{
  "protocol_version": "1.0",
  "event_id": "019abc13-...",
  "instance_id": "019abc12-...",
  "type": "activity_completed",
  "ts": "2026-02-16T10:00:00Z",
  "payload": {
    "activity_id": "019abc14-...",
    "name": "fetch",
    "result": {"rows": 1000},
    "logs": [{"stream": "stdout", "chunk": "..." }],
    "artifacts": [],
    "metrics": {"duration_ms": 243, "exit_code": 0}
  }
}
```

### Object Call Request/Result

```json
{
  "protocol_version": "1.0",
  "object_id": "019def34-...",
  "method": "increment",
  "input": {"by": 1},
  "idempotency_key": "019f00aa-..."
}
```

```json
{
  "protocol_version": "1.0",
  "object_id": "019def34-...",
  "method": "increment",
  "result": {"count": 42},
  "metrics": {"duration_ms": 8},
  "error": null
}
```

## Error Model

Every failure maps to a typed error:

- `code`: stable machine-readable code (`timeout`, `quota_exceeded`, `sandbox_exec_failed`, etc.)
- `message`: human-readable detail
- `retryable`: boolean
- `details`: optional structured payload

## Guarantees

- Activities and alarms are **at least once**.
- Object method execution is serialized one-at-a-time per object.
- Durable runtime provides idempotency keys for retried calls.
- Replay is deterministic from event history, not stack/closure serialization.

## Determinism Helpers

SDKs expose deterministic context helpers (with language-idiomatic naming), and the protocol supports recording/replaying their values:

- Time helper (for example `ctx.now()`)
- Random helper (for example `ctx.random()`)
- UUID helper (for example `ctx.newUuid()`)

Replay-safe orchestration code should use these helpers instead of direct wall-clock/random APIs.

## Durable Storage Model

Local durable state is persisted by the daemon in SQLite (`state.db`) for atomic transitions and efficient query/filter operations used by API and UI surfaces.

## Versioning

- `protocol_version` is required in every request and event.
- Minor versions are additive.
- Breaking changes require new major version.
- History migration tooling is required before major upgrades.

## Encoding

JSON is the canonical format for API and persisted history. Protobuf mirrors can be added later for high-throughput paths, generated from the same schema definitions.

## Related Docs

- [Durable Functions](durable-functions.md)
- [Durable Objects](durable-objects.md)
