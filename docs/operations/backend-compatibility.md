# Backend compatibility CI

Compatibility checks are intentionally layered. Pull requests run the
deterministic checks and the short local-container smoke contract; jobs that
need a dedicated host, a Kubernetes control plane, or hosted credentials run on
a schedule or by explicit dispatch.

## Validation matrix

| Surface | Supported target | Workflow or test | Trigger | Owner | Gate |
| --- | --- | --- | --- | --- | --- |
| Docker | Current GitHub-hosted Docker | `backend-compatibility.yml`, `container_backend_smoke` | Pull request and weekly | Runtime maintainers (@thrashr888) | Blocking |
| Podman | Current Ubuntu Podman | `backend-compatibility.yml`, `container_backend_smoke` | Pull request and weekly | Runtime maintainers (@thrashr888) | Blocking |
| Apple Containers | macOS 26+, Apple silicon, pinned 1.2.2 CLI | `apple-container-compatibility.yml` | Pull request, weekly, and approved live dispatch | macOS backend maintainer (@thrashr888) | Hosted contract blocking; live lifecycle dedicated-host gate |
| Firecracker | 1.16.1 with the 6.18.45 guest kernel | `firecracker-kvm-smoke.yml` | Weekly and approved dispatch | Runtime maintainers (@thrashr888) | Dedicated-host gate |
| Hyperlight | `hyperlight-wasm` 0.14, Linux x86_64 KVM | `hyperlight.yml` | Pull request and weekly | Runtime maintainers (@thrashr888) | Blocking |
| Kubernetes | 1.34 and 1.35; automatic 1.36 probe | `kubernetes-compatibility.yml` | Pull request and weekly | Orchestrator maintainers (@thrashr888) | 1.34/1.35 blocking; 1.36 activates when published |
| Nomad | 1.10.5 and 2.0.4 | `nomad-compatibility.yml` | Pull request and weekly | Orchestrator maintainers (@thrashr888) | Blocking |
| PostgreSQL, MySQL, Redis, Valkey | PostgreSQL 17, MySQL 8.4, Redis 7, Valkey 9.1 | `store-compatibility.yml` | Pull request, weekly, and dispatch | Storage maintainers (@thrashr888) | Blocking |

The Rust 1.89 feature matrix remains the cheap compile signal for optional
backends. The normal CI workflow owns formatting, linting, unit tests, and
release builds; these compatibility workflows do not change release artifacts
or release triggers.

## State preservation capabilities

"Snapshot", "stop/start", and "full-state pause" are separate contracts.
Filesystem and provider snapshots preserve selected files or disks and start
new processes on restore. Full-state pause preserves guest memory and the point
of execution.

| Backend | Existing snapshot/restore | Full-state pause/resume/fork |
| --- | --- | --- |
| Firecracker 1.16.1, Linux x86_64 KVM | No Firecracker-specific cold-snapshot contract | Supported only on an exactly compatible host after the native KVM gate passes |
| Docker / Podman | Container-filesystem image where implemented | Unsupported |
| Apple Containers | Exported container filesystem rebuilt as an image | Unsupported |
| Hyperlight | None | Unsupported |
| Kubernetes / Nomad | No portable AgentKernel full-state format | Unsupported |
| Daytona / Runloop / E2B / Modal | Provider-specific workspace, disk, or snapshot semantics | Unsupported unless a future provider contract explicitly proves process-memory continuity |
| Agent Computer | Custom bridge contract only | Unsupported |

Unsupported full-state operations must fail before mutating the sandbox or
creating artifacts. The CLI and MCP return an explicit capability error; the
HTTP API maps the same condition to `422 Unprocessable Entity`. There is no
automatic fallback to a filesystem snapshot. Users can request that weaker
behavior explicitly with the existing `snapshot take` and `snapshot restore`
commands.

Discover this contract through `full_state_pause_resume` and
`full_state_fork` in `GET /backends`; only Firecracker reports them as true.
Do not infer full-state support from the legacy `snapshots` or `resume`
booleans, which do not promise that memory or live processes survive. See
[Firecracker full-state lifecycle](firecracker-full-state.md) for the artifact,
compatibility, upgrade, and test contract.

## Hosted provider contracts

`scripts/test/provider-sdks.test.mjs` checks the maintained SDK exports and runs
the shared JSON-over-stdio bridge contract against deterministic fake clients for
Daytona, Runloop, E2B, and Modal. The fake contract covers create, status,
foreground command execution, file write/read, directory creation, stop/resume,
snapshot/restore, endpoint discovery, and destroy. It runs in pull-request CI
with `npm test` and never reads provider credentials.

Credentialed coverage is deliberately separate in
`provider-live-smoke.yml`. Dispatching it requires selecting one provider and
checking the cleanup confirmation. The smoke uses a unique, short-lived
resource name, has a 20-minute timeout, and always attempts destroy in the test
`finally` block. A second `always()` cleanup step scans the runner-temporary
resource manifest so a failed assertion cannot leave a sandbox behind. Provider
credentials are GitHub Actions secrets only; they are never passed to a pull
request from an untrusted fork.

To run a live check, dispatch the workflow from a trusted branch, select a
provider, and set `confirm_cleanup` to true. If a provider's account has a
different spending or retention policy, the provider owner must update the
workflow timeout and cleanup procedure before enabling it.

## Explicit non-blocking boundaries

GitHub-hosted Apple-silicon runners do not expose nested virtualization, so the
blocking pull-request lane installs Apple's signed 1.2.2 package, verifies the
macOS/architecture contract, runs structured-output tests, and compiles the
lifecycle smoke. The real lifecycle is an opt-in dispatch on the access-controlled
`agentkernel-apple-containers` self-hosted runner. The macOS backend maintainer
(@thrashr888) owns that runner and must run the live lane before releases that
change the Apple backend.

Kubernetes 1.36 does not yet have a published `kindest/node:v1.36.0` image. The
matrix probes that exact target on every run and automatically executes the full
lifecycle when it appears; 1.34 and 1.35 remain blocking today. Orchestrator
maintainers (@thrashr888) own the probe and must make 1.36 a required target in
the same change that first turns the probe green.

The Hyperlight native arm64 probe is an intentionally non-blocking backend
compatibility exception. It is limited to the known `hyperlight-host` 0.14 arm64
compiler failure, has an expiry check on **2026-11-30**, and is owned by the
runtime maintainers (@thrashr888). It must become blocking when a published
Rust-1.89-compatible arm64 `hyperlight-wasm` release is available.

The Firecracker lane uses an explicitly labelled, access-controlled self-hosted
runner because GitHub-hosted runners do not expose the required KVM device and
guest assets. The lane is scheduled and manually dispatchable; it is not silently
treated as evidence on ordinary pull requests. Runtime maintainers (@thrashr888)
own runner health and asset refreshes. A release that claims full-state
pause/resume/fork must additionally demonstrate RAM-only process continuity,
two concurrent divergent forks with independent writable disks, vsock
reconnection, corrupt-artifact failure, and cleanup on this native runner. The
current macOS development host and ordinary GitHub-hosted CI cannot supply that
evidence.

The Kubernetes and Nomad orchestrators provide internal service discovery for
declared sandbox ports: Kubernetes creates a per-sandbox ClusterIP Service,
while Nomad registers native services against the generated dynamic-port
labels. These are cluster-internal registrations only. Public domains,
Ingress/gateway resources, service meshes, TLS, and endpoint resolution remain
operator-owned concerns and are not inferred or created by AgentKernel.
