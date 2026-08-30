# RFC 0002: Exceptional Claude Code and Anthropic Integration

**Status**: Draft

**Author**: Paul Thrasher (@thrashr888)

**Date**: 2026-08-26

**Last updated**: 2026-08-29

**Tracking epic**: `agentkernel-mkfk`

**Working-backwards PRFAQ**: [AgentKernel for Claude](0002-claude-code-anthropic-integration-prfaq.md)

## Decision requested

Approve the following product direction and authorize **Milestone 0 only**:

1. Ship a native AgentKernel plugin for Claude Code as the first deliverable.
2. Develop a separate hosted Claude Agent mode in which the Claude Agent SDK
   runs inside an AgentKernel sandbox.
3. Use one generic `AgentSession` resource, with a Claude-specific binding, for
   lifecycle, policy, credentials, telemetry, and receipts across both modes.
4. Make the long-lived AgentKernel service the only authoritative owner of an
   agent session and its sandbox mutations.

Milestone 0 proves the unresolved identity, control, and security contracts. It
ends in two separately approvable child designs: companion mode and hosted
Claude Agent mode. Approval of this RFC does not authorize implementing the
entire roadmap or advertising a supported Anthropic partnership.

The following are **provisional** until Milestone 0 completes:

- the supported mechanism binding a Claude Code session to its plugin MCP
  process without model cooperation;
- the final `AgentSession` wire schema;
- the remote MCP authentication and transport profile; and
- whether hosted mode can use ordinary cold stop/resume for general availability
  while Firecracker full-state pause/fork remains preview.

## Summary

AgentKernel will become a customer-controlled execution and governance layer
for Claude agents through two deliberately separate products:

| Mode | Where Claude runs | AgentKernel's role | First audience |
|---|---|---|---|
| Claude Code companion | On the developer's host, IDE, or SSH session | Native plugin and explicit sandbox tools | Individual and enterprise developers |
| Hosted Claude Agent | Inside an AgentKernel sandbox through the Claude Agent SDK | Process supervisor, workspace, egress, credential, policy, and audit boundary | Platform and security teams |

Companion mode does not replace Claude Code's built-in Bash sandbox. Anthropic
does not document a pluggable sandbox-provider interface for Claude Code. The
plugin gives Claude explicit AgentKernel tools and makes the boundary visible.

Hosted mode implements the high-isolation architecture described in Anthropic's
Agent SDK deployment guidance: the complete agent process runs in the isolation
boundary, credentials remain outside it, and outbound traffic passes through a
policy-controlled proxy. The first hosted credential profile is the direct
Anthropic API using an API-key binding. Bedrock, Google Cloud, and Microsoft
Foundry require provider-native signing or token brokers and are follow-on
designs, not variants of static header injection.

An integration is exceptional only when it is native to Claude's supported
extension model, honest about its isolation, coherent across public APIs,
diagnosable, reversible, and safe under failure. “Claude can call a generic MCP
server” is the existing baseline, not the outcome.

## Why now

AgentKernel already has most of the raw components:

- a Claude adapter and tested Claude Code image;
- loose Claude skill and command files plus a generic stdio MCP server;
- local and remote sandbox backends;
- AgentKernel session metadata and Firecracker lifecycle work;
- host-side secret proxying;
- Cedar authorization and audit logging;
- signed command receipts; and
- HTTP, OpenAPI, desktop, and five public SDK surfaces.

They do not yet form one dependable product:

| Surface | Current gap |
|---|---|
| Distribution | The installer copies loose `.claude/` and `.mcp.json` files; it does not manage the native plugin package or version lifecycle |
| MCP | The custom server returns mostly text, pins an old protocol version, truncates output at 16 KiB, and has drifted from HTTP and SDK contracts |
| Sessions | A session stores a sandbox and agent label, not a Claude conversation, supervisor, policy, usage, or receipt chain |
| Ownership | MCP, CLI, and HTTP can create independent managers; no one process serializes all agent-session mutations |
| Credentials | The documented full-isolation path passes `ANTHROPIC_API_KEY` into the guest even though a proxy pattern exists |
| Approvals | The requesting MCP identity can currently reach permission-grant behavior; that is not independent human approval |
| Policy | Provider/model actions are not fully enforced and runtime context is frequently recorded as `unknown` |
| Receipts | Receipts describe individual commands, can include environment strings, and do not establish an enterprise trust root |
| Backends | MCP cannot negotiate the capabilities required for Claude, and current proxy routing is not portable across backends |
| Diagnostics | Installation is inferred from file existence rather than a plugin, MCP, service, backend, and policy handshake |

Anthropic's public architecture aligns closely with AgentKernel's strengths.
The Agent SDK runs the same agent loop and tools as Claude Code through a
supervised `claude` subprocess. Its hosting guidance treats the subprocess,
workspace, and transcript as durable local state. Its secure-deployment guide
explicitly discusses Firecracker, networkless guests, `vsock`, host-side egress
proxies, and credential injection outside the agent boundary. Claude Code
plugins package skills, hooks, and MCP servers and can be distributed through
public or private marketplaces.

## Relationship to existing plans

This RFC supersedes the Claude-specific direction in
`plan/05-agent-integration.md`, `plan/06-agent-in-sandbox.md`, and
`plan/plugin-install.md`. Those documents remain historical references for
generic agent integration, PTY work, and installer mechanics.

`plan/0001-platform-modernization.md` remains authoritative for AgentKernel
toolchains, generic agent images, backends, hosted sandbox providers, and public
SDK runtime support. This RFC owns Claude Code and Claude Agent SDK compatibility
and must update RFC 0001 if its generic compatibility matrix changes.

Existing access-policy, enterprise-policy, secure-transport, and Firecracker
lifecycle plans remain independently authoritative. Beads is the source of
truth for implementation state. Child beads are created only after reviewers
approve the direction and Milestone 0 decomposes the work.

## Goals

1. Install a valid, namespaced Claude Code plugin through supported Claude
   plugin and marketplace mechanisms.
2. Give companion users explicit AgentKernel sandbox tools with visible backend,
   workspace exposure, policy, and receipt status.
3. Host the Claude Agent SDK inside AgentKernel without putting the direct
   Anthropic API key in the guest, logs, receipts, snapshots, or traces.
4. Establish one authoritative `AgentSession` owner and one versioned wire
   contract across CLI, HTTP, OpenAPI, MCP, SDKs, and desktop.
5. Enforce real principal, tenant, provider, model, tool, workspace, backend,
   credential, and network context before hosted mode ships.
6. Keep Claude's native permissions and telemetry behavior intact while adding
   an independent AgentKernel authorization and evidence layer.
7. Make install, update, rollback, diagnosis, and removal predictable.
8. Produce partner-quality validation and real design-partner usage rather than
   relying on repository activity as adoption evidence.

## Non-goals

- Replacing or transparently intercepting Claude Code's built-in Bash, Read,
  Write, or Edit implementations.
- Treating Claude hook input or permission UI events as cryptographic proof of an
  independent human approval.
- Offering `claude.ai` login, subscription rate limits, or OAuth credential
  synchronization without prior Anthropic approval.
- Calling Docker or Podman hardware isolation.
- Shipping Bedrock SigV4, Google OAuth/ADC, or Microsoft identity through the
  initial static-header proxy.
- Building incremental copy-on-write host workspace synchronization in this
  RFC. That is a separate subsystem and child design.
- Promising deterministic model replay.
- Making Firecracker full-state lifecycle stable before native KVM safety and
  recovery gates pass.
- Rebranding an Agent SDK-based AgentKernel product as Claude Code.

## Alternatives considered

| Alternative | Decision | Reason |
|---|---|---|
| Keep the generic MCP integration only | Reject | It lacks native distribution, lifecycle identity, credentials, policy coherence, and supportability |
| Ship companion mode only | Viable first release, not the full direction | Fastest adoption path but does not isolate the Claude process or satisfy hardened multi-tenant hosting |
| Ship hosted mode only | Reject as first release | Stronger boundary but much larger security and operations surface; loses the easiest partner and user feedback loop |
| Rewrite every Claude Bash command through a hook | Reject | Not a supported sandbox-provider contract and does not relocate built-in file tools |
| Pass API keys into the guest | Reject | Easier, but violates the primary security and acquisition differentiation |
| Build an Anthropic-only runtime core | Reject | Claude-specific UX should use a generic `AgentSession` and capability contract where possible |

## Decisions and invariants

### The AgentKernel service is authoritative

`agentkernel serve` becomes the sole owner of agent-session state and mutations.
It serializes lifecycle operations and owns:

- the `AgentSession` store and operation journal;
- sandbox bindings and resolved backend capabilities;
- Cedar evaluation and independent approval records;
- credential and egress-proxy bindings;
- receipt chains and audit events; and
- trace/session correlation.

CLI, MCP, SDK, desktop, hooks, and hosted supervisors are clients. They do not
construct independent managers for agent-session mutations. Local companion
mode uses an authenticated private control transport, preferably a Unix-domain
socket with peer verification and a per-session capability token. Hosted and
remote clients use the authenticated HTTP control plane.

This does not require all ordinary one-shot AgentKernel commands to become
daemon-only. It applies to the new agent-session contract, whose identity and
durability cannot be reconstructed by unrelated short-lived processes.
Every sandbox bound to an AgentSession carries a durable ownership marker.
CLI, MCP, or legacy code that encounters that marker must delegate the operation
to the owning service. Only unbound sandboxes may use a direct short-lived
manager path.

### The public resource is generic

The public resource is `AgentSession`, not `ClaudeSession`. It has a
provider-specific `ClaudeBinding` when `agent.kind == "claude"`. This avoids a
Claude-only core while still representing Claude conversation and runtime
semantics explicitly.

The provisional model includes:

| Area | Required fields |
|---|---|
| Identity | AgentSession ID, name, principal, tenant, creation and activity timestamps |
| Agent | Kind, mode, plugin/SDK/CLI versions, Claude Code or Agent SDK conversation `session_id` when available |
| Components | Separate health/state for control service, supervisor, transcript, workspace, proxy, and each sandbox binding |
| Sandboxes | One primary binding plus optional child/sibling bindings with backend and assurance reason |
| Workspace | Ownership mode, source identity, revision/digest, exposure, dirty state, and artifact references |
| Anthropic | Provider endpoint profile, resolved model/effort where observable, and compatibility record |
| Governance | Policy revision, decision IDs, independent approval IDs, credential-binding references, limits |
| Observability | Per-turn trace links, usage summary, latest event cursor, and receipt-chain reference |

The model supports multiple turns and traces. It does not pretend one global
trace ID describes a long-running session.

### Security invariants

1. Requested assurance is a hard requirement; AgentKernel never silently falls
   back to a weaker backend or host execution.
2. Hosted mode has no credential values in the guest or persisted evidence.
3. An agent cannot approve its own privileged request.
4. Policy is evaluated by the authoritative service at every public entry point
   and again at the credential proxy.
5. Companion mode clearly distinguishes host Claude operations from
   AgentKernel sandbox operations.
6. Uninstall never deletes sandboxes, transcripts, workspaces, or receipts.
7. Claims derive from tested backend capabilities, not marketing aliases.

## Product 1: Claude Code companion

### Installation and distribution

The plugin uses the native Claude layout:

```text
integrations/claude-code/agentkernel/
├── .claude-plugin/plugin.json
├── .mcp.json
├── skills/
├── hooks/hooks.json
└── scripts/
```

Plugin-root `settings.json` is not a general configuration channel. Supported
plugin settings are used only within Anthropic's documented limits. AgentKernel
configuration uses plugin `userConfig`, ordinary user/project settings, or
administrator-controlled managed settings as appropriate.

The repository publishes a marketplace entry whose source is pinned by git SHA
or archive SHA-256. Claude does not enforce custom minimum/maximum-version
manifest fields, so compatibility is negotiated by the AgentKernel handshake
and published in a separate machine-readable compatibility record.

```bash
# Project scope
agentkernel plugin install claude

# User scope
agentkernel plugin install claude --global

agentkernel plugin status claude
agentkernel plugin update claude
agentkernel plugin uninstall claude
agentkernel doctor --integration claude
```

The CLI manages only scopes that an ordinary user controls. Enterprise
administrators distribute or force-enable the plugin through managed settings
and private marketplace policy; ordinary uninstall cannot override that policy.

Existing loose `.claude/` and `.mcp.json` installations receive a merge preview.
User-authored content is preserved. File existence is not treated as proof of a
healthy installation.

### Claude Code behavior

The plugin provides namespaced skills such as:

```text
/agentkernel:status
/agentkernel:run-isolated --workspace project-rw npm test
/agentkernel:receipt last
/agentkernel:diagnose
```

The first AgentKernel tool call creates or resolves an on-demand AgentSession
and sandbox. Creating a sandbox at `SessionStart` is deferred until the
Milestone 0 binding and latency spike is complete.

`SessionStart` attaches context only. It cannot enforce fail-closed behavior,
and an MCP hook may not yet be connected. Enforced companion policy uses
`PreToolUse` or `PermissionRequest` for supported blocking behavior, plus the
authoritative AgentKernel service. `ConfigChange` is audit-only for the scopes
Anthropic reports; it cannot block managed-policy changes and may not fire for
server-managed, MDM, or registry updates.

Claude Code permission decisions improve UX but remain advisory to AgentKernel.
Hook stdin and a same-user local helper can be forged. A privileged AgentKernel
operation therefore requires either:

- a separate AgentKernel desktop/terminal approval surface bound to the service;
- an authenticated remote administrator; or
- a pre-existing Cedar grant issued by an independent principal.

### MCP contract

Local companion mode uses plugin-bundled stdio MCP. Remote enterprise MCP is
provisional pending Milestone 0 transport and authentication work.

The first tool set is intentionally small:

| Group | Tools |
|---|---|
| Health | integration and backend capability status |
| Session | ensure, inspect, stop, and retain the bound AgentSession |
| Execution | one-shot and persistent sandbox execution, status, logs, and stop |
| Files | bounded read/write and artifact references |
| Evidence | retrieve and verify receipts; explain policy decisions |

Input and result behavior is gated by the supported Claude Code and MCP version
matrix. Where supported, privileged tools carry Anthropic's documented
`_meta["anthropic/requiresUserInteraction"]` annotation, but the server still
enforces independent approval. Progress, tasks, cancellation, structured
content, and resource behavior ship only after compatibility tests prove the
exact client behavior.

Large output follows Claude Code's documented result-size and automatic
file-reference behavior, including `_meta["anthropic/maxResultSizeChars"]`
where supported. AgentKernel removes its own current silent 16 KiB truncation
and returns an artifact reference or explicit size error.

The requesting MCP identity loses `permission_grant`. Fast pooled execution
uses the same policy and receipt path as persistent execution and cannot ignore
profile, backend, or compatibility inputs.

### Companion workspace boundary

The first companion release uses existing, explicit workspace modes rather than
inventing an incremental sync engine:

- no project mount for commands that do not need source;
- a read-only project mount for analysis; or
- an explicitly selected read-write project mount for builds and tests.

The default is no project mount. Project installation may record read-only or
read-write consent after showing the exact host path and exposure. Otherwise a
command that needs the repository returns `AK_WORKSPACE_CONSENT_REQUIRED` with
the configuration command; it does not infer consent from Claude's request. The
short `/agentkernel:run-isolated npm test` form works only after that project
default is configured.

Status and receipts disclose that a read-write project mount exposes the
selected host repository to sandbox code. AgentKernel still protects the rest
of the host, but it does not claim the mounted project is isolated from writes.
A hardened copy-on-write sync, conflict, diff, and apply workflow requires a
separate RFC with symlink, submodule, LFS, ignored-file, dirty-tree, and human
approval semantics.

## Product 2: Hosted Claude Agent

### Supervisor contract

Hosted mode runs an Agent SDK supervisor inside the sandbox. The supervisor owns
one Agent SDK client and its `claude` subprocess, communicates with the
AgentKernel service over an authenticated control channel, and streams typed
turn events. It is not an interactive Claude Code TUI.

The UX distinguishes three operations:

```bash
# Start infrastructure and the Agent SDK supervisor
agentkernel session start --name issue-123 --agent claude --mode hosted

# Send a turn and stream Agent SDK events
agentkernel session turn issue-123 --prompt "Fix the failing parser test"

# Open a sandbox shell for diagnosis; this is not the Claude conversation UI
agentkernel session shell issue-123
```

Cold resume and full-state resume are separate:

- **Cold resume** starts a new supervisor/CLI process, restores the sandbox
  workspace, and resumes the Claude conversation through its documented
  `session_id` and `SessionStore`.
- **Full-state resume** restores a paused Firecracker process and memory image.
  It remains preview until native KVM, runtime supervision, storage lineage,
  and proxy-rehydration gates pass.

Transcript health and workspace health are independent. A transcript mirror
failure does not masquerade as sandbox failure, and an infrastructure resume
does not claim the Claude conversation resumed until the SDK confirms it.

The first hosted workspace is sandbox-owned and pre-staged by the trusted
control plane before the networkless agent starts. Results leave through the
control plane as an explicit patch or artifact. Applying a patch to a host
checkout is outside the requesting agent's authority unless independently
approved. Guest-initiated Git clone/push, dependency downloads, remote MCP, and
arbitrary HTTP tools are outside the initial preview; each requires an explicit
policy-aware transport instead of direct guest egress.

### Credential and egress profile

The initial hosted profile supports the direct Anthropic sampling API only.
`ANTHROPIC_BASE_URL` is documented for sampling requests; it is not assumed to
cover every external tool or cloud provider.

The required flow is:

1. The networkless guest sends a credentialless sampling request through a
   backend-specific relay.
2. The authoritative proxy authenticates the AgentSession and sandbox.
3. Cedar validates tenant, endpoint, provider, model, limits, and policy.
4. The proxy strips guest-supplied credential headers, injects the bound key,
   records redacted metadata, and forwards the request.

Current Firecracker plumbing does not yet provide this path: its `vsock` service
is the guest-agent protocol, while the existing credential proxy is TCP. A
`vsock` HTTP relay, authenticated session binding, custom CA/routing behavior,
and crash-safe proxy rehydration are explicit hosted-mode prerequisites.

The preview also provides a bounded OTLP relay over the authenticated control
path when telemetry is enabled. General HTTP(S), Git, artifact, and remote-tool
relays are follow-on capabilities with destination and credential policy. Until
then, network-dependent guest tools fail explicitly.

Bedrock requires SigV4, Google Cloud requires provider-native OAuth/ADC, and
Microsoft Foundry requires Azure-specific identity. Each needs its own broker
and conformance design. They are ineligible until implemented; AgentKernel does
not place their raw credentials in the guest as a fallback.

Non-default `ANTHROPIC_BASE_URL` and third-party providers can bypass Anthropic
server-managed settings. Hosted images must therefore receive endpoint-managed
settings, SDK-supplied managed settings, or a documented Claude apps gateway
configuration. The integration reports which policy source is active.

No backend is eligible for hosted mode merely because it can start a container.
Its authenticated proxy transport, network boundary, credential isolation,
workspace persistence, tenant fencing, and deletion behavior must pass the
hosted conformance suite. Remote hosted providers are deferred until those
capabilities exist.

## Policy and backend assurance

Minimum authoritative identity, provider/model policy, independent approval,
and tenant fencing are prerequisites for hosted preview, not an enterprise
afterthought.

Every decision includes, where applicable:

- authenticated principal, tenant, and workload identity;
- AgentSession, Claude conversation, sandbox, and operation IDs;
- plugin, Agent SDK, Claude CLI, and protocol versions;
- provider endpoint, resolved model, limits, and region;
- tool, normalized command, path class, and network destination;
- repository/workspace identity and exposure mode;
- backend, assurance, image digest, and resource limits; and
- credential references and prior independent approvals.

Claude's permission system and AgentKernel policy may both deny. Neither can
weaken the other. The AgentKernel service and proxy enforce Cedar on every
path. The existing `UseLlmProvider` action becomes real enforcement before
hosted preview.

| Backend | Companion use | Hosted eligibility |
|---|---|---|
| Firecracker | Supported after current local lifecycle gates | First high-assurance target after native KVM plus `vsock` relay, proxy recovery, and secret-canary tests |
| Apple Containers | Local companion sandbox target | Later target after authenticated proxy and egress parity tests |
| Docker/Podman | Development and explicit container isolation | Ineligible until portable authenticated proxy transport passes; no hard-coded bridge assumption |
| Kubernetes/Nomad | Remote companion control where already supported | Follow-on runtime-class-specific conformance, identity, network policy, and tenant fencing |
| Hosted sandbox providers | Existing generic capability only | Deferred; current proxy-secret capability is insufficient |
| Hyperlight | One-shot compatible workloads only | Not eligible for full Claude Agent runtime |

Backend resolution returns the selected backend, assurance level, and reason.
If no backend satisfies every hard requirement, the request fails with a
capability explanation. There is no silent downgrade.

## Wire contract and compatibility

OpenAPI 3.1 is the canonical **wire** contract for AgentSession HTTP resources.
Generated wire models are mapped to handwritten Rust domain types; Rust domain
types are not a second wire definition. MCP schemas and SDK wire models derive
from the same versioned definitions, with CI fixtures proving semantic parity.
Handwritten clients may add idiomatic helpers but may not redefine fields.

The initial HTTP resource family is `/v1/agent-sessions`. Exact endpoints,
schemas, and stable error codes are a Milestone 0 output rather than frozen in
this umbrella RFC. Every asynchronous mutation returns an operation ID and is
idempotent. A client disconnect does not cancel a committed lifecycle change.

The compatibility record publishes tested ranges for AgentKernel, the plugin,
Claude Code, the Agent SDK, MCP behavior, hosted image, and architecture. The
handshake rejects incompatible versions with an exact recovery action. Git SHA
or archive digest pins distribution integrity; compatibility is not inferred
from unrecognized plugin-manifest fields.

### Existing session compatibility

The AgentSession store is additive and versioned; it does not reinterpret the
existing files managed by `src/session.rs`. During beta, existing
`agentkernel session start/list/stop/save/resume/delete` behavior remains the
legacy default when `--mode` is absent and prints a migration notice. New
companion or hosted sessions require `--mode` and use the authoritative service.
`session turn` and `session shell` apply only to the new resource.

HTTP `/sessions` continues to mean terminal recordings; the new wire resource
uses `/v1/agent-sessions`. SDK sandbox-session cleanup wrappers retain their
existing meaning. A future `agentkernel session migrate` must preview the
sandbox binding it can preserve and must not invent a Claude conversation ID.
Rollback leaves the new versioned store untouched for a newer binary to resume;
an older binary ignores it rather than downgrading records.

## Telemetry and receipts

When explicitly enabled and configured, Claude's native telemetry can continue
exporting to the customer's collector. Hosted Agent SDK and `claude -p` flows
can use documented inbound
`TRACEPARENT` propagation. Interactive companion Claude Code does not promise
that behavior; Milestone 0 must validate a separate hook/MCP correlation bridge
or use Claude session IDs without claiming native W3C parentage.

AgentKernel records:

- low-cardinality resource attributes such as service, version, backend class,
  assurance class, and environment; and
- high-cardinality session, sandbox, tenant, policy-decision, operation, trace,
  and receipt IDs on spans or logs, never metrics.

Prompt text, model output, tool arguments, source contents, and secrets are
excluded by default. Content capture requires explicit Claude and AgentKernel
data policy.

Receipts form a signed chain across turns and tool operations. They reference
session, sandbox, image, workspace, policy, approval, credential binding,
destination, outcome, usage, and artifact digests without embedding secrets.
Enterprise verification uses a configured trust root, not a public key supplied
only by the receipt being verified.

Offline verification proves that a trusted AgentKernel evaluator signed the
recorded policy input and decision; it does not by itself prove the decision was
correct. Historical re-evaluation is a separate operation and requires the
immutable policy bundle, schema, evaluator version, entities, context, and
decision input to be retained or retrievable by digest.

Replay has bounded meanings:

1. verify signatures, chain, recorded policy-decision evidence, and artifacts
   without execution;
2. reproduce selected tool executions in a compatible environment and issue a
   new linked receipt; or
3. fork supported workspace or full-state lineage into a new AgentSession.

None promises identical model output.

## Failure semantics

| Failure | Required behavior |
|---|---|
| Optional companion service unavailable | Claude Code continues; AgentKernel tools report degraded status |
| Managed companion policy requires AgentKernel | Supported blocking hooks or managed launcher deny relevant operations; `SessionStart` alone is never treated as enforcement |
| Plugin/protocol mismatch | Refuse incompatible AgentKernel operation and keep the last known-good plugin available |
| Backend cannot meet assurance | Explain missing capability; never run on host or a weaker backend |
| Credential proxy unavailable | Hosted session remains stopped or failed; key passthrough is not a fallback |
| Policy or independent approver unavailable | Follow explicit fail-closed policy; no agent self-grant |
| Transcript mirror failure | Surface transcript degradation separately from sandbox health |
| Client disconnect during mutation | Continue under the operation ID and allow idempotent status/retry |
| Full-state proxy rebind fails | Keep the VM paused and require recovery or explicit cold restart |
| Plugin uninstall | Remove only user-controlled registration; retain all session data and report it |

## Workstreams and milestones

The current tracking epic is `agentkernel-mkfk`. Milestone 0 creates child beads
and dependency links only after this direction is approved.

| Order | Workstream | Approval boundary | Outcome |
|---:|---|---|---|
| 0 | Contract, identity, and threat-model spike | This RFC | Decision-ready companion and hosted child designs |
| 1 | Native Claude Code companion | Separate child design | Versioned plugin beta with coherent MCP/session UX |
| 2 | Authoritative policy, approval, and proxy substrate | Separate platform design if needed | Enforceable identity and direct-Anthropic credential isolation |
| 3 | Hosted Claude Agent | Separate child design | Firecracker hosted preview with cold conversation resume |
| 4 | Enterprise and backend expansion | Per-provider/backend designs | Managed deployment, additional credential brokers, supported runtime classes |
| 5 | Partner and GA hardening | Release decision | Audited, supported integration with design-partner evidence |

### Milestone 0 exit criteria

- Choose and prototype the authenticated local control transport and sole
  service ownership path.
- Prove a race-free Claude session-to-MCP binding without model cooperation.
- Complete one minimal networkless Agent SDK sampling request through the
  authenticated relay on native KVM and prove a secret canary is absent from the
  guest and persisted evidence.
- Produce the versioned OpenAPI AgentSession draft with component health,
  sandbox bindings, turn traces, transcript, usage, and receipt-chain references.
- Threat-model hooks, local peer identity, approval forgery, MCP self-grants,
  receipt environment leakage, and plugin supply chain.
- Specify the minimum supported Claude Code/Agent SDK/MCP versions and test
  current plugin validation.
- Produce separate companion and hosted implementation estimates and beads.

### Companion beta exit criteria

- Native plugin validates and installs, updates, rolls back, and uninstalls at
  user-controlled scopes.
- A new user completes an isolated, signed test run in under five minutes
  without editing JSON.
- Status distinguishes host Claude operations, sandbox backend, workspace
  exposure, policy, and evidence.
- Privileged AgentKernel operations cannot be approved by the requesting MCP
  identity or forged hook input.
- Existing loose installations migrate without overwriting user content.

### Hosted preview exit criteria

- The Agent SDK supervisor, transcript store, cold resume, event stream, and
  sandbox shell have distinct tested contracts.
- Firecracker runs a networkless guest with a real `vsock` sampling relay on
  native x86_64 KVM.
- The direct Anthropic key remains absent from guest environment, `/proc`, disk,
  snapshot, transcript, receipt, log, trace, crash output, and artifacts.
- Identity, tenant fencing, provider/model Cedar policy, independent approval,
  and proxy enforcement are active before the first sampling request.
- Proxy state recovers safely after service restart; unsupported full-state
  transitions remain preview or fail explicitly.

### Shared general-availability gates

- Supported live platform and version matrix is green on current release
  artifacts.
- Contract parity holds across CLI, HTTP, OpenAPI, MCP, maintained SDKs, desktop,
  and documentation fixtures.
- Independent security review has no unresolved critical or high-severity
  findings.
- Plugin, image, binary, and provenance artifacts are signed and rollback is
  tested.
- Security, data handling, support, migration, recovery, and known limitations
  are published.

### Companion general-availability gates

- Supported Claude Code versions pass native plugin, hook, MCP, upgrade, and
  uninstall tests on macOS and Linux.
- Workspace exposure requires recorded consent and is visible in status and
  receipts.
- Optional and managed-enforcement failure modes behave exactly as documented.

### Hosted general-availability gates

- Native KVM evidence covers the supervisor, sampling and OTLP relays, cold
  conversation resume, policy, credential canaries, crash recovery, and cleanup.
- Every advertised hosted backend passes the same authenticated proxy, identity,
  tenant, workspace, and deletion conformance contract.
- Provider-specific credential brokers are advertised only after their own live
  security gates pass.

## Outcome metrics

These measure product success after technical gates; they are not substitutes
for release correctness.

- At least 95% successful plugin installations in supported environments.
- Median install-to-first-verified-run below five minutes.
- At least 80% of design-partner users complete a second AgentKernel-backed
  Claude session within 30 days.
- Three paid enterprise design partners use the integration on real repositories.
- At least 99.5% successful MCP control operations excluding policy denials and
  user cancellation.
- Zero credential-canary findings and zero silent assurance downgrades in the
  supported conformance matrix.

## Release and rollback

1. Ship the companion plugin through an explicit beta channel first.
2. Retain the previous plugin and compatibility record until update health
   checks pass.
3. Keep legacy loose-file support for one documented migration window.
4. Promote hosted mode backend by backend after its live security gates pass.
5. Roll back plugin, control service, and hosted image independently.
6. Never delete session data as part of software rollback or uninstall.
7. Treat a revoked plugin or image digest as a policy denial with a documented
   migration path.

## Risks

| Risk | Mitigation |
|---|---|
| Anthropic changes plugin, SDK, hook, or authentication behavior | Tested version ranges, scheduled current-version lane, adapter boundary, changelog review |
| Users think AgentKernel replaces Claude's sandbox | Explicit dual-mode UX and status; no transparent-replacement claim |
| Same-user plugin hooks forge approval | Treat them as advisory; require an independent AgentKernel principal |
| Hosted proxy scope expands into insecure cloud-provider emulation | Direct Anthropic first; separate provider-native broker designs |
| Backend claim exceeds its real transport or isolation | Capability conformance and hard failure instead of fallback |
| Hosted work absorbs a new workspace synchronization product | Sandbox-owned workspace first; separate hardened-sync RFC |
| Receipts imply deterministic AI replay | Distinguish verification, tool reproduction, and session fork |
| Open-source plugin can be forked | Strategic value comes from audited runtime, policy, operations, partner fit, customers, and support |

## Open questions

1. Which documented Claude Code mechanism can bind `SessionStart` input to the
   plugin's MCP process under concurrent sessions in one project?
2. Should the local companion service use the existing private control socket,
   a new session broker socket, or one unified authenticated UDS protocol?
3. Which AgentKernel approval surface is the first independent principal:
   desktop, terminal prompt owned by the service, or remote administrator?
4. What exact endpoint-managed settings profile is required when hosted sampling
   uses `ANTHROPIC_BASE_URL` and server-managed settings do not apply?
5. Is cold conversation resume sufficient for hosted GA while full-state
   process resume remains preview?
6. Which Anthropic marketplace or partner review is required before describing
   the integration as validated rather than compatible?

## References

### Anthropic

- [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview)
- [Hosting the Agent SDK](https://code.claude.com/docs/en/agent-sdk/hosting)
- [Securely deploying AI agents](https://code.claude.com/docs/en/agent-sdk/secure-deployment)
- [Agent SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks)
- [Agent SDK session storage](https://code.claude.com/docs/en/agent-sdk/session-storage)
- [Agent SDK observability](https://code.claude.com/docs/en/agent-sdk/observability)
- [Plugins in the Agent SDK](https://code.claude.com/docs/en/agent-sdk/plugins)
- [Claude Code plugins](https://code.claude.com/docs/en/plugins)
- [Claude Code plugin reference](https://code.claude.com/docs/en/plugins-reference)
- [Claude Code hooks](https://code.claude.com/docs/en/hooks)
- [Claude Code MCP](https://code.claude.com/docs/en/mcp)
- [Claude Code managed MCP](https://code.claude.com/docs/en/managed-mcp)
- [Claude Code sandboxing](https://code.claude.com/docs/en/sandboxing)
- [Claude Code settings](https://code.claude.com/docs/en/settings)
- [Claude Code server-managed settings](https://code.claude.com/docs/en/server-managed-settings)
- [Claude Code authentication](https://code.claude.com/docs/en/authentication)
- [Claude Code monitoring](https://code.claude.com/docs/en/monitoring-usage)
- [Claude Code plugin marketplaces](https://code.claude.com/docs/en/plugin-marketplaces)
- [Claude Code Desktop](https://code.claude.com/docs/en/desktop)

### AgentKernel

- `claude-plugin/.claude/skills/agentkernel/SKILL.md`
- `docs/agents/claude.md`
- `docs/features/secrets.md`
- `src/agents.rs`
- `src/mcp.rs`
- `src/plugin_installer.rs`
- `src/session.rs`
- `src/receipt.rs`
- `src/policy/cedar.rs`
- `src/backend/firecracker.rs`
- `src/vmm.rs`
- `api/openapi.yaml`
