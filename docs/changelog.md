
# Changelog

All notable changes to agentkernel are documented here.
See [GitHub Releases](https://github.com/thrashr888/agentkernel/releases) for downloadable binaries.

---

## v0.4.0 — API Surfaces & SDK Updates

_Unreleased_

### Added

- **File operations API** — read, write, and delete files inside running sandboxes via `PUT/GET/DELETE /sandboxes/{name}/files/{path}`
- **Batch execution API** — run multiple commands in parallel via `POST /batch/run`
- **Sandbox logs API** — retrieve audit log entries via `GET /sandboxes/{name}/logs`
- **Resource limits** — set `vcpus` and `memory_mb` when creating sandboxes
- **Security profiles via API** — pass `profile` (permissive/moderate/restrictive) on sandbox creation
- **SDK support** for all new endpoints across Node.js, Python, Rust, Go, and Swift SDKs
- **OpenAPI spec** updated to 0.4.0 with full schema coverage
- Terminal size detection for session recording
- Domain config validation (`DomainConfig.is_allowed()`)
- Command policy enforcement and attach session recording

### Fixed

- Fully-qualified tap name for `brew services`
- MkDocs internal links now use directory URLs
- Idempotency check for GitHub Packages publish

### Docs

- SDK documentation pages updated with file ops, batch, and logs examples
- Session recording, audit events, and config validation docs
- Integration levels and native sandbox links for all agents
- SDK links and TypeScript example on docs home and README

---

## [v0.3.1](https://github.com/thrashr888/agentkernel/releases/tag/v0.3.1) — Setup Auto-Installs Agent Plugins

_January 30, 2026_

### Fixed

- `agentkernel setup` now auto-installs agent plugins
- Crates.io OIDC token handling for publish workflow

**Full Changelog**: [v0.3.0...v0.3.1](https://github.com/thrashr888/agentkernel/compare/v0.3.0...v0.3.1)

---

## [v0.3.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.3.0) — Agent-in-Sandbox & SDKs

_January 30, 2026_

### Added

- **Agent-in-sandbox** with PTY support, environment variable passthrough, and example images ([#1](https://github.com/thrashr888/agentkernel/pull/1))
- **Client SDKs** for Node.js, Python, Rust, Go, and Swift
- **Agent plugins** for Claude Code, OpenCode, Codex, and Gemini CLI
- **Plugin installer** — `agentkernel plugin install` command
- **Homebrew service** — `brew services start agentkernel`
- **SSE streaming** — `/run/stream` endpoint for real-time command output
- **Audit logging** for all sandbox operations
- **Session recording** in asciicast v2 format with `agentkernel replay`
- **OpenAPI 3.1 spec** for the HTTP API
- Docker image to ext4 rootfs conversion for Firecracker
- Seccomp profile support for Docker backend
- Domain and command filtering config
- OIDC trusted publishing for npm, PyPI, and crates.io
- Comparisons page and benchmarks documentation
- MkDocs documentation site with Material theme

### Changed

- Default port changed from `8080` → `8880` → `18888`
- Claude plugin moved to agent-native paths

### Fixed

- CI Rust bumped to 1.88 for let-chains stabilization
- Missing sandbox backend handling in tests

**Full Changelog**: [v0.2.0...v0.3.0](https://github.com/thrashr888/agentkernel/compare/v0.2.0...v0.3.0)

---

## [v0.2.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.2.0) — Multi-Backend & Hyperlight

_January 22, 2026_

### Added

- **Unified Sandbox trait** for all backends (Docker, Podman, Firecracker, Apple Containers, Hyperlight)
- **Hyperlight WebAssembly backend** for sub-millisecond sandboxes (~68ms startup, ~3,300 RPS)
- **Apple Containers backend** for macOS 26+ with native container support
- **Daemon mode** with Firecracker VM pool for persistent fast execution
- **Container pool** for 5.8x faster ephemeral runs
- **WAT support** — WebAssembly text format compilation
- **File operations** on the Sandbox trait and `agentkernel cp` command
- **`--backend` CLI flag** for backend selection
- Vsock connection caching and single-RPC daemon exec
- Per-agent pool configuration and MCP skill docs
- Agent compatibility modes with preset profiles
- Dockerfile support with auto-detection and caching
- `[[files]]` config section for file injection at startup
- AllBeads onboarding for issue tracking

### Performance

- Docker/Podman optimized with direct `run --rm` for ephemeral execution
- Apple containers optimized with single-operation ephemeral runs
- Hyperlight sandbox pooling with `warm_to()` for precise pre-warming

**Full Changelog**: [v0.1.2...v0.2.0](https://github.com/thrashr888/agentkernel/compare/v0.1.2...v0.2.0)

---

## [v0.1.2](https://github.com/thrashr888/agentkernel/releases/tag/v0.1.2) — Container Pooling & Firecracker Exec

_January 21, 2026_

### Added

- **Container pool** for pooled vs non-pooled execution comparison
- **Persistent exec channel** for Docker backend
- **Guest agent** wired up for Firecracker exec via vsock
- Firecracker vsock support via Unix socket protocol

### Performance

- 110ms boot time achieved (89% faster) with i8042 disable
- Optimized Firecracker boot args for 35% faster startup

### Fixed

- Proper KVM permission detection (not just existence check)
- Docker image to Firecracker runtime auto-mapping
- Rootfs ownership after Docker build
- Setup improvements for new users

**Full Changelog**: [v0.1.1...v0.1.2](https://github.com/thrashr888/agentkernel/compare/v0.1.1...v0.1.2)

---

## [v0.1.1](https://github.com/thrashr888/agentkernel/releases/tag/v0.1.1) — Security Hardening & Performance

_January 20, 2026_

### Security

- **Input validation** — sandbox names, runtime names, and Docker images validated against strict patterns
- **Command injection** — fixed potential injection via sandbox names and Docker filters
- **Path traversal** — prevented directory traversal in rootfs resolution
- **SBPL injection** — validated paths used in macOS Seatbelt profiles
- **TOCTOU fixes** — atomic operations for socket cleanup

See [SECURITY.md](https://github.com/thrashr888/agentkernel/blob/main/SECURITY.md) for the full security policy.

### Performance

Docker backend **33% faster**:

| Metric | Before | After |
|--------|--------|-------|
| Total (10 sandboxes) | 6.70s | 4.50s |
| Avg start | 258ms | 174ms |
| Avg stop | 172ms | 109ms |

- Removed redundant container existence checks
- Added `--rm` flag for automatic cleanup
- Combined stop+remove into single operation
- 1-second stop timeout for ephemeral containers

### Docs

- [BENCHMARK.md](https://github.com/thrashr888/agentkernel/blob/main/BENCHMARK.md) with measured results and methodology

**Full Changelog**: [v0.1.0...v0.1.1](https://github.com/thrashr888/agentkernel/compare/v0.1.0...v0.1.1)

---

## [v0.1.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.1.0) — Initial Release

_January 20, 2026_

### Features

- **Firecracker microVM management** — create, start, stop, remove, and exec in isolated VMs
- **Sub-125ms boot times** — lightweight ~25MB images with minimal Linux kernel
- **Multiple runtimes** — base, Python, Node, Rust, Go with auto-detection
- **Security profiles** — permissive, moderate, and restrictive isolation levels
- **MCP server** — Claude Code integration via JSON-RPC over stdio
- **HTTP API** — programmatic access for automation
- **macOS support** — Seatbelt sandbox fallback, Docker KVM host for nested virtualization
- **Cross-platform** — Linux (native KVM) and macOS (Docker Desktop)

**Full Changelog**: [v0.1.0](https://github.com/thrashr888/agentkernel/commits/v0.1.0)
