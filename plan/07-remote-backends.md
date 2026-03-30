# 07 Remote Backends

## Summary

Add hosted remote backends for `daytona`, `runloop`, `e2b`, and `agentcomputer` under the same `Sandbox` contract used by local backends.

This implementation introduces:

- new backend enum values and CLI parsing
- persisted remote runtime metadata (`remote_id`, `remote_metadata`, `workspace_revision`, `endpoints`)
- a shared `RemoteSandbox` backend implementation
- a JSON-over-stdio Node bridge at [`scripts/remote-bridge.mjs`](../scripts/remote-bridge.mjs)
- managed `/workspace` sync for remote backends
- structured endpoint exposure in the HTTP API and `agentkernel info`

The current bridge includes a fully testable `mock` mode so the remote substrate works end-to-end in local development. Provider-specific API bindings can plug into the same bridge protocol without changing the Rust-side contract.

## Public Surface

- `BackendType` now includes `daytona`, `runloop`, `e2b`, and `agentcomputer`.
- `agentkernel.toml` now supports:
  - `[remote]`
  - `[remote.daytona]`
  - `[remote.runloop]`
  - `[remote.e2b]`
  - `[remote.agentcomputer]`
  - `[remote.profiles.<name>]`
- Sandbox state now persists:
  - `remote_id`
  - `remote_namespace`
  - `remote_metadata`
  - `workspace_revision`
  - `endpoints`
- HTTP sandbox responses now include `endpoints` and `workspace_revision`.

## Implementation Notes

- Remote backends use `create_sandbox_with_state()` so reconnect paths can hydrate from persisted provider identity instead of sandbox name alone.
- `mount_cwd` is widened into managed sync for remote backends:
  - push on start
  - push before exec/attach
  - pull after exec/attach/stop
  - fail with a sync conflict when local or remote workspace content diverges from the last synced revision
- Remote backends reject unsupported host-coupled features early:
  - host volume mounts
  - proxy-based secret bindings
  - host home-directory mounts
  - SSH exposure
- The included bridge script is intentionally substrate-focused. It owns provider lifecycle, workspace sync, and service endpoint publication. The Rust core owns the `Sandbox` trait and persisted state.

## Validation

- `cargo fmt --all`
- `cargo test --quiet`
