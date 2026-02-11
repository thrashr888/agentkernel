
# Changelog

All notable changes to agentkernel are documented here.
See [GitHub Releases](https://github.com/thrashr888/agentkernel/releases) for downloadable binaries.

---

## [v0.10.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.10.0) — Browser Automation & Desktop App

_February 10, 2026_

### Added

- **Browser automation SDK** — `BrowserSession` abstraction across all 5 SDKs (Python, Node.js, Go, Rust, Swift); high-level `goto()`, `screenshot()`, `evaluate()` methods that orchestrate Playwright inside sandboxes
- **MCP browser tools** — 5 new tools (`browser_create`, `browser_goto`, `browser_screenshot`, `browser_evaluate`, `browser_remove`) that collapse the 4-step manual orchestration into single tool calls
- **MCP image content type** — `browser_screenshot` returns native MCP image content (`type: "image"`, PNG) instead of text; new `ToolOutput` enum separates text and image responses in the MCP dispatcher
- **MCP output truncation** — tool responses capped at 16KB with head(8KB) + tail(8KB) preservation; images bypass truncation
- **Tauri 2 desktop app** — full macOS desktop application with React 19/TypeScript frontend and Rust backend via Apple Containers
- **Desktop sandbox management** — create, start, stop, remove sandboxes; streaming exec with real-time output; file browser with read/write support
- **Desktop Quick Run** — one-click sandbox execution from the dashboard
- **Desktop terminal button** — launch terminal sessions into running sandboxes
- **Desktop snapshots** — take, list, restore, and delete snapshots from the UI
- **Desktop diagnostics** — system health checks and backend status in Settings
- **Desktop activity toasts** — real-time notifications for sandbox operations
- **Desktop agent quickstart** — launch Claude Code, Gemini CLI, Codex, Copilot CLI, Amp, and Pi directly from the app
- **Desktop audit log** — view sandbox operation history
- **Desktop secrets** — manage secrets from the UI
- **Desktop GC & export** — garbage collection and config export from Settings
- **Desktop container logs** — view container stdout/stderr in sandbox detail
- **Desktop template profiles** — pre-configured sandbox templates
- **Desktop policy page** — Cedar policy engine status, interactive policy check tester, reload button
- **Desktop policy log page** — dedicated page for policy decision audit trail with action and decision filter dropdowns, pagination
- **Desktop sandbox table** — column sorting, status filter buttons with counts (running/stopped/total), search by name/image/IP
- **Desktop sandbox detail** — copy-to-clipboard for sandbox name
- **Desktop app CI** — 3-job GitHub Actions workflow (`app-ci.yml`): frontend typecheck + build, Tauri Rust lint + test, macOS cross-compile (ARM64 + x64) with artifact upload
- **Copilot agent support** — `AgentType::Copilot` adapter for GitHub Copilot CLI; plugin with MCP JSON; example Dockerfile and config
- **Policy HTTP endpoints** — `POST /policy/reload` and `GET /policy/audit` for policy engine management
- **Browser automation templates** — `playwright` and `playwright-stealth` built-in templates (Python 3.12, 2GB RAM, Chromium/Firefox/WebKit)
- **SSH policy action** — `ssh` now accepted in policy check endpoints and CLI
- **Shared browser scripts** — `src/browser_scripts.rs` module with Playwright script constants shared between MCP tools and future HTTP API endpoints
- **POST /sandboxes/:name/start** — HTTP endpoint to start a stopped sandbox
- **Docs** — desktop app page, browser automation and GitHub Copilot agent added to mkdocs nav

### Changed

- **Desktop UI** — black & white Helvetica aesthetic with dark mode support
- **Desktop app icon** — custom agentkernel icon
- **Desktop templates page** — added "Browser Automation" category ordering

### Fixed

- **Agent API key leak** — API keys were injected into sandboxes even when `pass_env=false`; now guarded by security profile
- **Shell injection in Apple backend** — `write_file_unchecked` interpolated paths into `sh -c`; now uses positional arguments
- **`is_local_image()` too broad** — matched all `agentkernel-*` images; tightened to only `agentkernel-snap:` snapshot tags
- **`import_image_from_docker` child process** — `docker save` child was not waited on; now properly awaited with exit status check
- **Snapshot `ls` unchecked** — `ls -1 /` exit status was not checked in `take_apple`; now fails explicitly on error
- **Agent install command mismatch** — CLI used `@google/gemini-cli` and `npm install opencode`; aligned with desktop (`@anthropic-ai/gemini-cli`, `cargo install opencode`)
- **Apple Containers backend** — opaque toast backgrounds, snapshot `--pull=never`, Tauri IPC parameter alignment
- **Clippy warnings** — resolved across `http_api.rs`, `vmm.rs`, `snapshot.rs`
- **Policy check SSH action** — fixed HTTP 400 when checking `ssh` action (was missing from match statement)
- **Enterprise config** — removed `[enterprise]` section from example `agentkernel.toml` (should not ship enabled by default)
- **Unused import** — removed dead `shlex` import in Python SDK browser module

---

## [v0.9.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.9.0) — Sandbox DX

_February 5, 2026_

### Added

- **Persistent volumes** — `agentkernel volume create <slug>`, `volume list`, `volume delete`; mount via `--volume slug:/path` on `create` or `run`; volumes persist across sandbox lifecycle
- **Custom image builder** — `agentkernel build -t name .` builds images from Dockerfile; `images local-list`, `images local-rm`; use built images with `create --image local:name`
- **TTL extension** — `agentkernel extend-ttl <sandbox> --by 1h` to extend sandbox lifetime; `POST /sandboxes/:name/extend` HTTP endpoint; `sandbox_extend_ttl` MCP tool
- **Snapshots via HTTP/MCP** — `GET/POST/DELETE /snapshots`, `POST /snapshots/:name/restore` HTTP endpoints; 5 MCP tools (`snapshot_list`, `snapshot_take`, `snapshot_get`, `snapshot_delete`, `snapshot_restore`)
- **SDK volume support** — all SDKs (Rust, Node.js, Python, Go, Swift) now support `volumes` in `CreateSandboxOptions`
- **Per-command exec options** — `agentkernel exec` now supports `--workdir` (`-w`) and `--sudo` flags; HTTP API and MCP `sandbox_exec` tool accept `workdir`, `env`, and `sudo` parameters
- **Git source cloning on create** — `agentkernel create --source git:URL [--git-ref REF]` clones a repo into `/workspace` at creation time; also available via HTTP API (`source_url`/`source_ref`) and MCP `sandbox_create`
- **Batch file write** — `POST /sandboxes/{name}/files` accepts `{"files": {"/path": "content"}}` for multi-file writes; MCP `sandbox_write_files` tool for the same
- **`ExecOptions` trait method** — `Sandbox::exec_with_options()` supports workdir, user, and env per-command across all backends
- **Detached commands** — run long-lived processes in the background with `agentkernel exec --detach`, retrieve logs with `exec-logs`, check status, kill, and list; HTTP API routes at `/sandboxes/{name}/exec/detach` and `/sandboxes/{name}/exec/detached/{id}`; 5 new MCP tools (`sandbox_exec_detach`, `sandbox_exec_status`, `sandbox_exec_logs`, `sandbox_exec_kill`, `sandbox_exec_list`)
- **SDK updates** — all four SDKs (Rust, Node.js, Python, Swift) now support exec options (`workdir`/`env`/`sudo`), git source cloning (`source_url`/`source_ref`), batch `writeFiles`/`write_files`, and detached commands (`execDetached`/`detachedStatus`/`detachedLogs`/`detachedKill`/`detachedList`)
---

## [v0.8.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.8.0) — Secure Transport

_February 3, 2026_

### Added

- **SSH certificate authentication** — ephemeral ed25519 certs with per-sandbox CA, sshd injection via `--ssh` flag, `agentkernel ssh` command for cert-authenticated shell access
- **SSH config generation** — `agentkernel ssh-config` outputs `~/.ssh/config` entries for VS Code Remote-SSH and other IDEs
- **SSH session recording** — `agentkernel ssh --record` captures sessions in asciicast v2 format
- **SSH ProxyCommand** — `agentkernel ssh-proxy` enables transparent SSH through `agentkernel` without manual port management
- **Vault SSH integration** — optional HashiCorp Vault CA for certificate signing instead of local per-sandbox CA
- **TLS for HTTP API** — rustls-based HTTPS with auto-generated self-signed certs or custom cert/key via `--tls-cert`/`--tls-key`
- **Container IP display** — `list`, `info`, HTTP API, and MCP output show Docker bridge IPs for running sandboxes
- **Port mapping** — `-p`/`--publish` flag for host:container port forwarding (e.g. `-p 8080:80`)
- **Transport security policy** — Cedar policy for SSH and TLS enforcement in enterprise mode
- **`[ssh]` config section** — `enabled`, `port`, `user`, `cert_ttl`, and Vault settings in `agentkernel.toml`

### Fixed

- **PTY allocation** — SSH certificates now include all 5 standard OpenSSH extensions (`permit-pty`, `permit-X11-forwarding`, etc.)
- **Audit log deserialization** — renamed `SshConnected.user` to `ssh_user` to fix `#[serde(flatten)]` field collision; legacy entries auto-repaired on read
- **SSH cert auth** — fixed account unlock, file ownership, sshd_config Alpine compatibility, and auto-assigned port resolution
- **`snapshot restore`** — moved restore under `snapshot restore` subcommand, fixed doc drift

### Security

- Certificates are ephemeral (default 30m TTL) and never stored long-term
- No password authentication — certificate-only SSH access
- Per-sandbox CA isolation — compromising one sandbox's CA doesn't affect others

**Full Changelog**: [v0.7.1...v0.8.0](https://github.com/thrashr888/agentkernel/compare/v0.7.1...v0.8.0)

---

## [v0.7.1](https://github.com/thrashr888/agentkernel/releases/tag/v0.7.1) — CI Fix

_February 2, 2026_

### Fixed

- **Docker build** — added templates directory to Docker build context

**Full Changelog**: [v0.7.0...v0.7.1](https://github.com/thrashr888/agentkernel/compare/v0.7.0...v0.7.1)

---

## [v0.7.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.7.0) — OSS CLI Roadmap

_February 2, 2026_

### Added

- **Templates** — `template list`, `template save`, `template add` (from GitHub), `template remove`; built-in templates for common runtimes
- **Snapshots** — `snapshot take`, `snapshot list`, `snapshot delete`, `snapshot restore` for checkpoint/restore workflows
- **Sessions** — `session start`, `session list`, `session stop`, `session save`, `session resume`, `session delete` for agent session management
- **Pipelines** — `pipeline` command for multi-step TOML-defined workflows with dependencies
- **Parallel execution** — `parallel` command for concurrent multi-job execution with `--job` syntax
- **Secrets** — `secret set`, `secret get`, `secret list`, `secret delete` for secure credential storage
- **Export/Import** — `export-config` and `import-config` for TOML-based sandbox portability
- **Filesystem export** — `export` command to save sandbox filesystem as tar
- **Garbage collection** — `gc` command for expired sandbox cleanup
- **Per-branch sandboxes** — `create --branch` auto-names from git project + branch

### Fixed

- Container naming consistency across commands
- Parallel job parsing and pipeline safety checks
- TTL edge cases in snapshot expiry

**Full Changelog**: [v0.6.0...v0.7.0](https://github.com/thrashr888/agentkernel/compare/v0.6.0...v0.7.0)

---

## [v0.6.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.6.0) — Enterprise Policy Engine

_February 1, 2026_

### Added

- **Cedar policy engine** — declarative authorization using [AWS Cedar](https://www.cedarpolicy.com/) with default-deny evaluation, role-based and attribute-based access control (`--features enterprise`)
- **JWT/OIDC identity** — authenticate users via JWT tokens with JWKS validation and OIDC device authorization flow for CLI login
- **Multi-tenant policy hierarchy** — organization and team scoping with inheritance for policy evaluation
- **Policy bundle signing** — Ed25519 signature verification with trust anchors, version rollback protection, and expiry enforcement
- **Policy cache** — offline operation with configurable modes (`default_policy`, `cached_only`, `cached_with_expiry`) and FNV-1a integrity hashing
- **Audit logging** — OCSF-compatible policy decision logs with structured JSON output and SIEM-ready event streaming
- **HTTP API policy endpoints** — `GET /policy/status`, `POST /policy/check`, `POST /policy/reload` for runtime policy management
- **CLI policy commands** — `policy status`, `policy check`, `policy audit-log` for local policy inspection
- **Policy enforcement in HTTP API** — all `/run`, `/create`, `/exec`, `/attach` endpoints enforce Cedar authorization
- **AgentKernelPolicy CRD** — namespaced Kubernetes Custom Resource for Cedar policies, managed via `kubectl apply` and GitOps (shortname: `akp`)
- **ClusterAgentKernelPolicy CRD** — cluster-scoped Cedar policy CRD for global rules (shortname: `cakp`)
- **K8s policy operator** — watches policy CRs, validates Cedar syntax, aggregates by scope and priority, hot-reloads the evaluation engine
- **Sandbox policy enforcement** — operator evaluates `Create` action against Cedar engine before creating pods, blocks denied requests with status update
- **Example Cedar policies** — default permit, RBAC, MFA-required, runtime restrictions, and org isolation examples in `examples/enterprise/`
- **Compliance mapping** — SOC 2, HIPAA, and FedRAMP control mapping documentation

### Changed

- **Default features** — `kubernetes`, `nomad`, and `enterprise` features are now included in default builds
- **`generate_crd_manifests()`** — returns `Vec<String>` instead of tuple, includes all 4 CRDs (sandbox, pool, policy, cluster-policy)
- **`run_operator()`** — accepts optional `CedarEngine` and `PolicyAuditLogger`, runs 3 controllers concurrently when enterprise is enabled

### Docs

- Kubernetes orchestration docs updated with policy CRD reference, evaluation order, identity annotations, and examples
- Enterprise policy examples README with K8s-native and GitOps workflow documentation
- Kubernetes example README with policy CRD quickstart

**Full Changelog**: [v0.5.1...v0.6.0](https://github.com/thrashr888/agentkernel/compare/v0.5.1...v0.6.0)

---

## [v0.5.1](https://github.com/thrashr888/agentkernel/releases/tag/v0.5.1) — Docker Images, Nomad Pack & Docs

_February 1, 2026_

### Added

- **Docker image publishing** — `ghcr.io/thrashr888/agentkernel:latest` and versioned tags built automatically on each release
- **Helm OCI publishing** — `oci://ghcr.io/thrashr888/charts/agentkernel` published automatically on each release
- **Nomad Pack** — configurable Nomad deployment via `nomad-pack run` with variables for count, backend, resources, and Consul service registration

### Fixed

- **Dockerfile** — bumped Rust from 1.83 to 1.88 (required for edition 2024 let-chains)
- **K8s labels** — replaced fictional `agentkernel.io/` domain prefix with bare `agentkernel/` in all labels and CRDs
- **Deploy docs** — replaced local file paths with `git clone` / `curl` from GitHub; added honest notes about OCI/Docker image availability
- **CI** — added `checks: write` permission for `rustsec/audit-check` workflow

### Changed

- **Docs restructure** — deploy content inlined into orchestration-kubernetes.md and orchestration-nomad.md; deploy.md slimmed to shared concerns only
- **README** — added Kubernetes and Nomad to platform table and orchestration section

**Full Changelog**: [v0.5.0...v0.5.1](https://github.com/thrashr888/agentkernel/compare/v0.5.0...v0.5.1)

---

## [v0.5.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.5.0) — Kubernetes & Nomad Orchestration

_January 31, 2026_

### Added

- **Kubernetes backend** — run sandboxes as Pods on any K8s cluster with NetworkPolicy isolation, optional gVisor/Kata RuntimeClass, and warm pool support (`--features kubernetes`)
- **Nomad backend** — run sandboxes as Nomad job allocations with Docker/exec/raw_exec drivers and Consul/Vault integration (`--features nomad`)
- **Kubernetes operator** — CRD types (`AgentSandbox`, `AgentSandboxPool`) and controller for declarative sandbox management
- **Warm pool managers** for both K8s (label-based warm→active) and Nomad (parameterized batch jobs) backends
- **Helm chart** for Kubernetes deployment (`deploy/helm/agentkernel/`)
- **Nomad job spec** for Nomad deployment (`deploy/nomad/agentkernel.nomad.hcl`)
- **Orchestrator config** — `[orchestrator]` section in `agentkernel.toml` for namespace, warm pool size, runtime class, and driver settings
- **Security mapping** for K8s Pod security contexts and Nomad cap_drop to existing permission profiles
- `remote_id` and `remote_namespace` fields on `SandboxState` for tracking cluster-side resources

### Performance

- **O(1) sandbox state detection** — batch state queries instead of per-sandbox checks across all backends
- Optimized K8s and Nomad backend latency (~570ms one-shot, faster with warm pools)

### Docs

- Orchestration documentation with separate Kubernetes and Nomad pages
- Benchmark results for K8s and Nomad backends
- Updated backend comparison table

### Fixed

- K8s and Nomad backend fixes from live integration testing
- CI `rustsec/audit-check` now has `checks: write` permission

**Full Changelog**: [v0.4.0...v0.5.0](https://github.com/thrashr888/agentkernel/compare/v0.4.0...v0.5.0)

---

## [v0.4.0](https://github.com/thrashr888/agentkernel/releases/tag/v0.4.0) — API Surfaces & SDK Updates

_January 30, 2026_

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

**Full Changelog**: [v0.3.1...v0.4.0](https://github.com/thrashr888/agentkernel/compare/v0.3.1...v0.4.0)

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
