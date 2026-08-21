# RFC 0001: Platform and Dependency Modernization

**Status**: Draft

**Author**: Paul Thrasher (@thrashr888)

**Date**: 2026-08-20

**Last updated**: 2026-08-20

**Tracking epic**: `agentkernel-zvvh`

## Summary

Modernize AgentKernel's compiler and JavaScript toolchains, sandbox backends,
hosted-provider adapters, base images, bundled agent CLIs, durable-store
clients, and public SDKs without turning the work into one high-risk dependency
upgrade.

The program keeps Rust 1.89 as the initial minimum supported Rust version
(MSRV), adds current-stable validation, moves supported JavaScript tooling to
maintained Node.js releases, and introduces an explicit compatibility matrix for
every backend and external service AgentKernel claims to support.

Work is divided into independently releasable milestones. Beads is the source
of truth for issue state; the progress table in this RFC is a human-readable
summary and must be updated when a milestone changes state.

## Motivation

AgentKernel spans more compatibility surfaces than a typical Rust CLI:

- the core Rust binary and optional feature sets;
- Docker, Podman, Apple Containers, Firecracker, and Hyperlight;
- Kubernetes and Nomad schedulers;
- Daytona, Runloop, E2B, and Modal hosted sandboxes;
- SQLite, PostgreSQL, MySQL, Redis, and Valkey stores;
- a React/Tauri desktop application;
- Node.js, Python, Go, Rust, and Swift SDKs; and
- images that bundle independently released AI-agent CLIs.

Several of these surfaces have drifted beyond ordinary patch maintenance.
Firecracker 1.7 is outside upstream support, the 6.1 guest kernel is nearing the
end of its supported window, current Kubernetes bindings do not cover all
supported Kubernetes releases, and several Rust transitive dependencies are
unmaintained or affected by soundness advisories. Node.js 20 and Alpine 3.20 are
also past their normal support windows, while Vite, Tailwind, and TypeScript
have major upgrades waiting.

The current test strategy does not reliably expose that drift. Docker
integration failures are allowed to pass, several optional backends are not
compiled in CI, hosted-provider code receives only a JavaScript syntax check,
and most service integrations have no versioned live test lane.

## Goals

1. Run every default runtime and backend on an upstream-supported release.
2. Publish compiler, runtime, backend, orchestrator, database, and SDK support
   policies that can be enforced in CI.
3. Remove known unmaintained, yanked, and unsound dependencies where a viable
   supported migration exists.
4. Validate hosted providers through one shared behavioral contract rather than
   provider-specific syntax checks.
5. Keep upgrades independently releasable and reversible.
6. Preserve existing user configuration and persisted sandbox metadata unless a
   separate migration explicitly changes them.
7. Make future maintenance routine through scheduled compatibility checks and a
   tested-version manifest.

## Non-goals

- Redesigning the sandbox API or JSON-over-stdio remote bridge protocol.
- Adding a new sandbox provider solely as part of this program.
- Automatically migrating user-selected images or runtime versions.
- Promising compatibility with every historical backend or service release.
- Combining every major dependency upgrade into one pull request.
- Making Hyperlight the default backend; it remains feature-gated until its
  execution model and platform coverage justify a separate decision.

## Current baseline and initial targets

Versions in this table are the planning baseline as of 2026-08-20. Exact patch
targets may advance before implementation, but the compatibility policy and
validation gates remain the same.

| Surface | Baseline | Initial target or policy |
|---|---|---|
| Core Rust | CI pinned to 1.89; no root `rust-version` | Declare MSRV 1.89 and test MSRV plus current stable |
| Rust SDK | MSRV 1.85 | Retain until an SDK dependency requires a deliberate increase |
| Desktop build | Vite 6, React 19, Tailwind 3, TypeScript 5 | Vite 8 first; Tailwind 4 and TypeScript 7 in separate changes |
| Node.js | Scripts CI on 22; examples and SDK docs include 20 | Minimum 22, primary CI and images on Node.js 24 LTS |
| Firecracker | 1.7 with Linux 6.1 guest kernel | Supported Firecracker 1.16.x and Linux 6.18 guest kernel |
| Apple Containers | 1.2.x compatibility work | Test current supported 1.2.x releases on Apple silicon |
| Hyperlight | `hyperlight-wasm` 0.12 | 0.14, feature compile in CI, Linux/KVM smoke test |
| Kubernetes | `kube` 0.98, `k8s-openapi` 0.24 | `kube` 4.x, `k8s-openapi` 0.28+, Kubernetes 1.34-1.36 |
| Nomad | Ignored integration tests | Nomad 1.10 LTS and 2.0 compatibility lanes |
| Hosted providers | Daytona 0.203, Runloop 1.14, E2B 2.18, Modal 0.7 | Supported SDKs plus shared contract tests and live smoke tests |
| Default Linux image | Alpine 3.20 | Maintained Alpine stable line; initial target 3.24 |
| Agent CLIs | Several unpinned installs; legacy Copilot package | Tested, recorded versions and supported Copilot package |
| Durable stores | Unit-heavy, little service-version coverage | Versioned SQLite, PostgreSQL, MySQL, Redis, and Valkey tests |

## Compatibility policy

### Compiler and language runtimes

- The root Rust package declares an MSRV and tests both that version and current
  stable. A dependency upgrade may raise the MSRV only through an explicit
  decision recorded in this RFC or a successor RFC.
- Node.js libraries declare the oldest supported active LTS release. CI tests
  that floor and the current LTS release.
- Python, Go, Swift, and Rust SDKs declare their compiler/runtime floors and test
  every claimed release or a documented representative matrix.
- A newer dependency is not, by itself, sufficient reason to drop a supported
  compiler or runtime.

### Local sandbox backends

| Backend | Required validation |
|---|---|
| Docker | Current stable lifecycle, exec, attach, file, port, volume, and pool tests |
| Podman | Current stable lifecycle and file-operation parity with Docker |
| Apple Containers | Current supported 1.2.x on macOS/Apple silicon, including structured output compatibility |
| Firecracker | Supported VMM and guest kernel on x86_64 KVM; lifecycle, networking, vsock, snapshot, and recovery tests |
| Hyperlight | Feature build on every PR and a Linux/KVM Wasm execution smoke test |

### Orchestrators

- Kubernetes support follows upstream-supported Kubernetes minor releases. The
  client bindings must be capable of representing the newest supported API.
- Nomad support covers the current LTS line and current major release.
- Orchestrator integration suites run on a schedule and before a release that
  changes the corresponding adapter.

### Hosted sandbox providers

Daytona, Runloop, E2B, and Modal implement one provider-neutral behavioral
contract covering:

- create, inspect/status, stop/resume, and destroy;
- foreground and detached command execution;
- file upload, download, push, and pull;
- snapshots and restore where the provider supports them; and
- endpoint/tunnel discovery.

Every pull request runs deterministic contract tests with fake SDK clients.
Credentialed smoke tests run on demand or on a schedule with timeouts, spending
limits, unique resource names, and cleanup that executes even after failure.
Provider-specific limitations are explicit capabilities, not silent no-ops.

### Images and bundled agent tools

- Default tags point to maintained releases. Existing explicit user image
  selections are not rewritten.
- Published or recommended agent images record the tested base image, agent CLI
  version, architecture, and build date.
- Image builds pin versions or immutable digests where practical. A scheduled
  refresh job may propose updates but does not silently change released images.
- Each agent image must build and pass a minimum smoke test: executable
  discovery, version output, help or non-network startup, expected runtime user,
  and writable workspace.

### Durable stores

- SQLite migrations and data round trips run in the normal test suite.
- PostgreSQL, MySQL, Redis, and Valkey run in service-backed CI against the
  documented supported versions.
- Client-library majors are upgraded separately from schema or behavior
  changes. Tests must cover legacy configuration and existing persisted data.

## Upgrade rules

1. Patch and compatible minor updates may be grouped when the full affected
   test surface passes.
2. Major migrations receive a dedicated bead and pull request unless two
   packages are inseparable, such as `kube`, `k8s-openapi`, and `schemars`.
3. A pull request must not mix unrelated frontend, backend, provider, and
   database migrations.
4. New defaults must not alter stored user choices.
5. A compatibility test cannot use `continue-on-error` after its backend is
   declared supported. Temporary exceptions require an owner, linked bead, and
   expiration condition.
6. Optional Cargo features must at least compile in CI. Supported optional
   backends also require an environment-appropriate smoke test.
7. Dependency warnings are evaluated by reachability and runtime risk, but an
   ignored advisory must carry a documented justification and follow-up bead.

## Workstreams and progress

The tracking epic is `agentkernel-zvvh`. Status values below summarize beads;
the bead itself is authoritative.

| Order | Workstream | Bead | Status |
|---:|---|---|---|
| 0 | Compatibility CI foundation | `agentkernel-zvvh.1` | Open |
| 1 | Firecracker and guest-kernel upgrade | `agentkernel-zvvh.2` | Open |
| 2 | Rust MSRV and dependency refresh | `agentkernel-kmwf` | Open |
| 2 | Kubernetes, Nomad, and Hyperlight modernization | `agentkernel-zvvh.3` | Open |
| 3 | Vite 8 desktop migration | `agentkernel-nl7s` | Open |
| 4 | Hosted-provider compatibility and contract tests | `agentkernel-2yhv` | In progress |
| 4 | Daytona package rename | `agentkernel-zx4z` | Open |
| 5 | Base images and bundled agent CLIs | `agentkernel-zvvh.4` | Open |
| 6 | Durable-store clients and service compatibility | `agentkernel-zvvh.5` | Open |
| 7 | SDK language/runtime support matrices | `agentkernel-zvvh.6` | Open |

When work changes one of these states, the implementing pull request updates
this table and the RFC's `Last updated` field. Detailed subtask state remains in
beads rather than being duplicated as Markdown task lists.

## Milestone sequence

### Milestone 0: Make compatibility visible

Build the layered validation system before most major upgrades:

- enforce the root MSRV and current-stable Rust lanes;
- compile default, no-default, and supported optional feature combinations;
- make Docker integration failures blocking after known flakiness is fixed;
- add Podman and Hyperlight lanes;
- add provider fake-contract tests;
- add scheduled KVM, Kubernetes, Nomad, and service-backed jobs; and
- define opt-in credentialed provider smoke tests.

This milestone may land incrementally, but every later milestone must add its
new target to the matrix before being considered complete.

### Milestone 1: Remove unsupported infrastructure

Upgrade Firecracker and its guest kernel first because they are the clearest
support-policy gap and sit beneath security-sensitive functionality. Validate
fresh installation and in-place upgrade paths on KVM hosts before changing the
default download metadata.

### Milestone 2: Modernize the Rust backend stack

Declare Rust 1.89 in the root package, refresh compatible patches under that
toolchain, then migrate coordinated backend groups:

1. `kube`, `k8s-openapi`, and `schemars`;
2. `hyperlight-wasm` and its Git dependencies;
3. direct `reqwest` 0.13 and TLS feature cleanup;
4. the MySQL patch line that removes obsolete `lru` and proc-macro transitives;
5. Redis, SQLite, TOML, and other majors behind focused tests; and
6. `sysinfo` only after a deliberate decision about its higher Rust floor.

Completion requires resolving or documenting every remaining `cargo audit`
warning reachable from a supported feature set.

### Milestone 3: Modernize the desktop toolchain

Move CI and packaging to Node.js 24 LTS while retaining Node.js 22 as the
declared floor where dependencies permit it. Upgrade Vite and its React plugin
together, remove the explicit esbuild minifier dependency in favor of Vite's
supported Oxc/Rolldown path, and validate both browser development and Tauri
production builds.

React/Tauri patch updates may accompany this work. Tailwind 4 and TypeScript 7
remain separate migrations with their own visual or type-system review.

### Milestone 4: Modernize hosted providers

Preserve the bridge protocol while upgrading each adapter independently:

- rename `@daytonaio/sdk` to `@daytona/sdk`;
- update Runloop after contract coverage confirms its lifecycle and snapshot
  calls;
- update E2B within its current major and validate reconnect/file behavior;
- migrate Modal file operations from the legacy `sandbox.open` interface to the
  current filesystem API; and
- split Agent Computer into its own bead if no production adapter exists.

### Milestone 5: Refresh runtime images and agent CLIs

Move new defaults away from Alpine 3.20 and Node.js 20. Replace the discontinued
`@githubnext/github-copilot-cli` package with the supported GitHub Copilot CLI.
Audit install users and paths, particularly OpenCode's root installation versus
runtime-user PATH. Record and smoke-test all bundled agent versions.

### Milestone 6: Modernize data services

Upgrade database clients only after service-backed tests exist. Cover Redis and
Valkey compatibility explicitly, validate PostgreSQL and MySQL TLS/configuration
paths, and test SQLite migrations against data produced by the current release.

### Milestone 7: Align public SDKs

Publish runtime floors for all SDKs, add matrix coverage, and coordinate HTTP,
SSE, and generated-model behavior across languages. SDK dependency majors may
move at a different pace from the core binary when that preserves a useful
runtime floor.

## Release and rollback strategy

- Each milestone lands in one or more focused pull requests and is releasable on
  its own.
- Backend and provider migrations retain compatibility parsing or configuration
  aliases for at least one release when feasible.
- Default changes are release-note items and include an explicit previous-value
  escape hatch.
- A failing optional live lane blocks releases that touch that integration, even
  if unrelated pull requests continue using deterministic contract tests.
- Rollback means reverting the focused migration and restoring the previous
  tested-version manifest, not reverting the entire modernization program.

## Completion criteria

This RFC is complete when:

- all child beads under `agentkernel-zvvh` are closed or explicitly deferred
  with rationale;
- no default backend or image relies on an upstream-EOL version;
- the root MSRV and SDK runtime floors are declared and enforced;
- every claimed backend and external service has a documented compatibility
  target and automated validation proportional to its cost;
- hosted-provider behavior is covered by shared contract tests;
- remaining dependency advisories have documented, time-bounded exceptions;
- released agent images publish tested tool versions; and
- the support matrix is included in release documentation.

## Risks

### Matrix cost and flakiness

Live backends and hosted services are slower and less deterministic than unit
tests. The mitigation is a layered matrix: deterministic tests on each pull
request, environment-specific integration jobs on a schedule, and credentialed
smoke tests only where they add unique evidence.

### Raising user requirements accidentally

Compiler, Node.js, image, and database upgrades can silently exclude existing
users. Floors are therefore explicit, tested separately from preferred
versions, and changed only with release notes and migration guidance.

### Provider API drift

Hosted SDKs, especially pre-1.0 packages, may break without a major version.
The shared adapter contract and tested-version manifest make drift observable
without coupling the public AgentKernel protocol to an SDK.

### Over-broad pull requests

Large lockfile changes can hide unrelated behavior changes. The workstream
boundaries and upgrade rules keep dependency graphs reviewable and rollback
practical.

## Open questions

1. Should the Node.js SDK floor be 22 immediately, or should one compatibility
   release retain Node.js 20 while examples and images move to 24?
2. Should supported-version metadata live in one machine-readable manifest that
   drives setup defaults, docs, and CI matrices?
3. Which hosted-provider smoke tests are safe to run nightly within agreed cost
   and cleanup limits?
4. Should Redis and Valkey be treated as one compatibility promise or two
   independently versioned targets?
5. Does the `sysinfo` upgrade justify raising the root MSRV above 1.89, or should
   it remain deferred?
6. After the compatibility matrix is reliable, should Hyperlight remain
   experimental or graduate to a documented supported backend?

## References

- [Rust releases](https://blog.rust-lang.org/releases/)
- [Node.js release schedule](https://nodejs.org/en/about/previous-releases)
- [Vite migration guide](https://vite.dev/guide/migration)
- [Firecracker release policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/RELEASE_POLICY.md)
- [Firecracker guest kernel policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md)
- [Kubernetes releases](https://kubernetes.io/releases/)
- [Nomad release notes](https://developer.hashicorp.com/nomad/docs/release-notes)
- [Alpine release branches](https://www.alpinelinux.org/releases/)
- [GitHub Copilot CLI installation](https://docs.github.com/en/copilot/how-tos/copilot-cli/cli-getting-started)
- [`plan/testing-plan.md`](testing-plan.md)
- [`plan/hyperlight-rfc.md`](hyperlight-rfc.md)
- [`plan/apple-containers.md`](apple-containers.md)
