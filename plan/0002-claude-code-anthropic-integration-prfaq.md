# PRFAQ: AgentKernel for Claude

**Status**: Draft working-backwards artifact

**Author**: Paul Thrasher (@thrashr888)

**Date**: 2026-08-29

**Source RFC**: [RFC 0002: Exceptional Claude Code and Anthropic Integration](0002-claude-code-anthropic-integration.md)

**Tracking epic**: `agentkernel-mkfk`

**Working names and channels**: “AgentKernel for Claude Code,” “Hosted Claude
Agent,” companion beta, and hosted preview remain subject to product,
trademark, support, and release review.

This document describes the customer experience AgentKernel intends to earn.
It is not a current availability announcement, an implementation
authorization, or evidence of an Anthropic partnership. RFC 0002 remains the
architecture source of truth and authorizes Milestone 0 only. The Claude Code
companion and Hosted Claude Agent require separate designs and release
decisions.

## Press release

The release below is target copy written from the future companion-beta launch
day. Hosted Claude Agent remains a separately gated direction, not part of that
launch.

### AgentKernel gives Claude Code a boundary you can see

**The native companion lets developers send selected builds, tests, and scripts
to policy-controlled sandboxes without pretending Claude Code itself moved off
the host.**

*[Future companion-beta launch date]* — AgentKernel today released the beta of
**AgentKernel for Claude Code**, a native companion that gives developers an
explicit, inspectable boundary for selected software work. It is the first
product in AgentKernel's broader Claude direction. **Hosted Claude Agent**, a
future mode for platform teams operating the Claude Agent SDK, is not part of
this launch and has its own design, security evidence, and release decision.

AI coding agents are most useful when they can build, test, inspect, and change
real software. Those are also the moments when an unclear boundary becomes
costly. Developers need to know whether a command ran on their laptop or in a
sandbox, whether their repository was exposed read-only or read-write, and
what verifiable execution evidence accompanies the result. For operations
explicitly routed through AgentKernel, security teams need enforceable identity,
workspace, network, and approval policy without turning every developer action
into a ticket. Claude Code's host-native operations remain outside that
boundary.

The Claude Code companion makes that boundary explicit. Claude continues to run
in the developer's normal Claude Code environment. When a developer or Claude
selects an AgentKernel operation, AgentKernel runs that work in a session-bound sandbox
and reports the resolved backend, assurance level, workspace exposure, policy
decision, and evidence status. A source-independent command can run with no
project mount. Repository analysis can use explicit read-only access. Builds
and tests can use explicit read-write consent. AgentKernel never infers that
consent from the model's request.

The result is a workflow that feels native to Claude Code rather than a folder
of copied configuration. The plugin installs, updates, diagnoses, rolls back,
and uninstalls through supported Claude plugin mechanisms. Namespaced commands
such as `/agentkernel:status`, `/agentkernel:run-isolated`, and
`/agentkernel:receipt` let developers see the boundary before work starts and
inspect a signed record afterward. Uninstalling the plugin does not delete
sandboxes, workspaces, transcripts, or receipts.

For organizations operating longer-running agents, AgentKernel is separately
designing Hosted Claude Agent. That future mode moves a supervised Claude Agent
SDK process into an AgentKernel-managed sandbox. Its initial preview is intended
to use a pre-staged, sandbox-owned workspace and a Firecracker guest with direct
general networking disabled. Sampling requests would cross an authenticated,
policy-controlled relay; the direct Anthropic API key would stay on the trusted
side of that boundary. The agent would return patches and artifacts through the
control plane instead of receiving arbitrary guest egress.

The companion beta uses one AgentKernel session control plane for lifecycle,
policy, approvals, and receipt chains. Hosted Claude Agent is designed to use
the same control plane for its credential and telemetry bindings if separately
approved. Claude's own permissions remain in effect. Privileged AgentKernel
operations cannot ship until a distinct authenticated principal, scoped
approval binding, expiry, and replay protection prove the requesting agent
cannot grant its own authority.

> “Claude should be able to use powerful tools without making the execution
> boundary a matter of faith,” said Paul Thrasher, creator of AgentKernel. “The
> boundary should be visible before a task runs, authorized before dispatch,
> enforced at controlled lifecycle and network boundaries, and inspectable
> afterward.”

Hosted Claude Agent will not enter preview until native-KVM credential,
identity, recovery, and tenant-isolation gates pass. Supported companion
versions, platforms, backends, and known limitations are published in a
machine-readable compatibility record.

*[Launch facts to complete only after release approval: beta channel; supported
Claude Code, AgentKernel, operating-system, and backend versions; install
command; first-run command; compatibility-record location; support channel.]*

## Customer FAQ

### Companion beta

#### What exactly is AgentKernel for Claude Code?

It is a native Claude Code companion plugin that gives Claude explicit
AgentKernel tools for isolated execution, session status, files, policy
explanations, and signed evidence. Claude Code itself continues to run on the
developer's host, IDE, or SSH environment. Selected AgentKernel operations run
inside AgentKernel sandboxes.

#### Does AgentKernel replace Claude Code's built-in sandbox or intercept every command?

No. Claude Code does not currently expose a documented pluggable sandbox
provider that AgentKernel can transparently replace. Claude's built-in Bash,
Read, Write, and Edit operations remain Claude Code operations. The companion
adds explicit, namespaced AgentKernel operations and clearly distinguishes them
from host-side work.

#### What can I do with the companion?

The first beta is designed for tasks such as running an unfamiliar build or
test suite, inspecting generated artifacts, using a persistent sandbox for a
debugging session, and retrieving evidence about what ran. The initial
AgentKernel tool set stays intentionally small: health, session, execution,
bounded file, and evidence operations.

#### Can the plugin access my repository automatically?

No. The default is no project mount. A developer can configure a read-only
mount for analysis or explicitly consent to a read-write mount for builds and
tests. AgentKernel shows the exact host path and exposure before recording a
project default. A command that needs ungranted access fails with a recovery
instruction instead of guessing.

#### Does a read-write mount protect my repository from sandbox code?

No. Code in the sandbox can modify the selected mounted repository. The
selected backend is intended and tested to restrict access outside that mount
according to its published capability and assurance profile; this is not an
absolute guarantee about the rest of the host. Status and receipts disclose the
boundary. A hardened copy-on-write synchronization and review workflow is a
different product decision and is not part of the first companion release.

#### What does “isolated” mean?

It means the selected backend satisfied the requested, tested capability and
assurance profile. AgentKernel reports the backend and the reason it qualified.
It does not call Docker or Podman hardware isolation, and it does not silently
fall back to host execution or a weaker backend when a hard requirement cannot
be met.

#### Does the companion copy my Claude credentials into its sandbox?

No. Claude keeps using its existing authentication outside the
AgentKernel sandbox; the plugin does not copy Claude credentials into an
AgentKernel guest.

#### Can Claude approve its own privileged AgentKernel action?

The beta is a no-go until the answer is demonstrably no. Claude permission
dialogs can improve the experience, but they are not an independent AgentKernel
authorization. A privileged action requires a distinct authenticated principal,
a scoped approval binding with expiry and replay protection, or administrator
policy granted before the agent's request. The final approval surface remains a
Milestone 0 decision.

#### What do receipts prove?

Offline verification proves signature and chain integrity for policy evidence
and referenced execution metadata or digests signed by a trusted AgentKernel
evaluator. A customer-configured verification key anchors that trust. A receipt
does not prove factual completeness, policy correctness, retention of raw
customer content, or that a model will produce the same answer again.
Historical policy re-evaluation requires the original immutable policy inputs
and evaluator metadata.

#### Does AgentKernel record prompts, model output, or source code?

Not by default. AgentKernel telemetry excludes prompt text, model output, tool
arguments, source contents, and secrets unless the customer explicitly enables
content capture under both Claude and AgentKernel data policy. Within telemetry,
session and policy identifiers may appear in traces or logs but not metric
labels. Policy, audit, and receipts may separately retain identities, decisions,
outcomes, content digests, and structured-secret-redacted forms of normalized
commands, path classes, and network destinations. Raw prompts, source, tool
arguments, and model output are not retained by default; redaction tests must
cover tokens and sensitive arguments embedded in commands or URLs.

#### Can AgentKernel replay a Claude session deterministically?

No. “Replay” has bounded meanings: verify a receipt chain without executing,
re-run selected tools in a compatible environment and issue a new linked
receipt, or fork supported workspace or VM lineage into a new session. It does
not promise identical model output.

#### What happens if AgentKernel is unavailable?

In an optional companion installation, Claude Code continues working and the
AgentKernel tools report degraded status. In a managed environment, supported
covered operations may be configured to fail closed. Claude Code's other
host-native operations remain outside the companion boundary.

#### How do upgrades, rollback, and uninstall work?

AgentKernel keeps the last known-good plugin and compatibility record until an
update passes health checks. The plugin and service can roll back independently.
Uninstall removes only user-controlled registration and never deletes session
data. Administrator-enforced installations remain under administrator control.

### Future direction: Hosted Claude Agent

#### What is Hosted Claude Agent, and why is it separate?

It is a future product for platform and security teams. The target is to run an
Agent SDK supervisor and its Claude subprocess inside an AgentKernel-managed
sandbox. The initial sandbox-owned workspace is the canonical work state; typed
turn events and a transcript mirror cross into the control plane under a
retention policy that must be defined before preview. It is not the interactive
Claude Code terminal UI and will not be marketed as “hosted Claude Code.”

The separation is a security fact, not packaging. Companion mode isolates only
work selected through AgentKernel tools while Claude remains on the host. Hosted
mode is designed to put the supported agent process and workspace inside the
sandbox while exposing only declared control-plane relays.

#### Why is hosted mode not available with the companion beta?

It still has to prove authenticated session and tenant binding, independent
approval, native-KVM isolation, crash-safe proxy recovery, and a real sampling
request from a guest with direct general networking disabled. The host proxy
necessarily handles the bound Anthropic key in trusted host memory and its
credential store; the guest sends a credentialless request.

Preview is a no-go unless no secret canary is found across the named test matrix
for guest environment, guest `/proc`, guest disk and snapshots, or persisted
transcripts, receipts, logs, traces, crash output, and artifacts. That is a
bounded test result, not a claim that leakage is universally impossible.

#### Is hosted mode air-gapped, and which providers does it support?

It is not air-gapped. “Direct general networking disabled” means Anthropic
sampling and, when enabled, bounded telemetry still cross explicit authenticated
relays. Each relay is part of the advertised policy and security profile.

The initial profile is direct Anthropic API sampling only. Bedrock signing,
Google OAuth or ADC, and Microsoft identity each need a provider-native broker
and separate security approval. Claude subscription authentication is out of
scope without Anthropic's approval. General guest HTTP, Git, downloads, and
remote tools are not silently routed around the boundary.

#### Can hosted work resume, and what happens when a dependency fails?

The initial target is cold conversation resume: restore the workspace, start a
new supervisor, and use the Agent SDK's supported session identity and store.
Full process-and-memory resume remains a separate Firecracker preview. If the
service, policy, authenticated relay, independent approver, or required backend
is unavailable, the hosted session remains stopped or failed. It never falls
back to a raw guest credential, undeclared egress, or weaker assurance.

### Availability and relationship

#### Is this an official Anthropic product, certification, or partnership?

No such claim is authorized. AgentKernel is an independent project designed to
work through documented Claude Code extension and Claude Agent SDK surfaces.
“Official,” “certified,” “Anthropic-approved,” and partnership language require
separate written authorization.

#### When will it be available, on which platforms, and at what price?

No date or price is committed by this PRFAQ. RFC 0002 currently authorizes only
the contract, identity, control-transport, and threat-model spike. The companion
beta and hosted preview each need a separate approval. Availability will name
exact Claude, AgentKernel, operating-system, architecture, and backend versions
that passed the release matrix.

## Internal FAQ

### 1. What customer problem are we solving?

Teams want Claude to perform consequential software work without losing track
of four facts: where it ran, what it could reach, who authorized it, and what
evidence remains. Today AgentKernel exposes useful pieces, but customers must
assemble loose plugin files, generic MCP tools, sandbox commands, policy, and
receipts themselves. The product turns those pieces into one supportable
journey.

### 2. What is the one-sentence promise?

**Companion:** See and control where operations explicitly routed through
AgentKernel execute, what workspace and network boundary they receive, whether
independent approval was required, and what verifiable evidence remains.

**Hosted direction:** Run a supported Claude Agent SDK workload inside a
governed AgentKernel boundary with declared relays, external credential custody,
tenant policy, and auditable outputs.

### 3. What does “exceptional” mean in concrete product terms?

An exceptional integration is:

- native to supported Claude distribution and extension mechanisms;
- honest about whether Claude itself or only a selected task is isolated;
- explicit about workspace, backend, assurance, credential, and network state;
- enforced by an authoritative service rather than cooperative prompt text;
- unable to let the requesting agent approve its own privileged work;
- coherent across CLI, HTTP, OpenAPI, MCP, SDKs, desktop, and evidence;
- diagnosable, reversible, compatible across named versions, and safe under
  partial failure; and
- validated by real design-partner use, not repository traffic alone.

### 4. Who is the first customer?

The companion's first customer is a developer or engineering team already using
Claude Code who wants a low-friction way to run selected builds, tests, and
unfamiliar code in a visible sandbox. The first enterprise customer is a
security-conscious engineering organization that needs managed policy,
independent approvals, and auditable evidence for covered AgentKernel operations
without taking Claude Code away from developers. Claude-native host operations
remain outside that companion policy boundary.

The hosted mode's first customer is a platform team building durable coding
agents for real repositories and willing to start with a narrow direct-Anthropic,
Firecracker, pre-staged-workspace profile.

Before beta positioning is finalized, design-partner discovery must narrow the
primary economic buyer and initial customer profile. “Claude developers and
enterprises” is not a usable sales or support definition.

### 5. Who is not the initial customer?

- Teams that require transparent interception of every built-in Claude Code
  tool.
- Customers who require Bedrock, Google Cloud, Microsoft Foundry, or Claude
  subscription authentication on day one.
- Workloads that require unrestricted guest internet, arbitrary remote tools,
  or unattended Git push in the first hosted preview.
- Teams that treat containerization as sufficient hardware isolation.
- Customers who require deterministic model replay or stable full-memory VM
  migration.

### 6. Why now?

AgentKernel already has a Claude adapter, generic MCP surface, sandbox backends,
sessions, a host-side credential proxy, Cedar policy, signed command receipts,
OpenAPI, SDKs, and active Firecracker lifecycle work. Anthropic exposes native
plugin and Agent SDK surfaces that let those capabilities become a coherent
product. The gap is no longer raw sandbox execution; it is identity, ownership,
distribution, transport, policy, evidence, and operational coherence.

The market-timing hypothesis is that agent capability is moving faster than the
execution and governance controls enterprises need for real repositories. That
is not yet validated demand. Before companion implementation expands, design
partners must show that explicit AgentKernel routing is useful enough to repeat
and that governed hosted execution is important enough to fund.

### 7. Why ship the companion before hosted mode?

The companion has the shortest path to real usage and validates distribution,
session binding, workspace consent, policy UX, receipts, compatibility, and
support with a smaller security and operations surface. Hosted mode adds a
supervisor, durable transcript, a sampling relay for guests with direct general
networking disabled, credential broker, tenant fencing, workspace staging,
recovery, and native-KVM validation. Starting there would delay customer
learning while bundling several independent risks.

### 8. Why build hosted mode at all?

Companion mode cannot claim that the complete Claude process is isolated. Some
platform and security teams need the agent process, workspace, network, and
credentials governed as one workload. Hosted mode is the high-assurance product
for that need and the strongest expression of AgentKernel's microVM, policy,
credential, lifecycle, and evidence advantages.

### 9. Why will customers pay instead of using Claude's sandbox, a generic MCP server, a container, a remote development environment, or direct Agent SDK hosting?

Claude's sandbox governs Claude's native host-side tools but does not create the
AgentKernel backend, lifecycle, policy, and evidence contract. MCP is one client
surface and does not by itself provide native installation, durable session
identity, a sole lifecycle owner, independent approval, credential isolation,
backend conformance, signed evidence, compatibility, rollback, or support. A
container or remote development environment can isolate a process but does not
automatically provide the same assurance negotiation, tenant-bound credential
relay, policy evidence, or cross-surface operations. Direct Agent SDK hosting
leaves the platform team to assemble and audit those systems.

Customers pay only if the integrated contract saves more platform and security
work than it adds to the developer path. That remains a design-partner
hypothesis; feature breadth alone does not validate willingness to pay.

### 10. What does the companion beta include?

The target beta includes a valid namespaced plugin, install/update/status/
rollback/uninstall flows, an authenticated local AgentKernel service, an
on-demand `AgentSession`, a small coherent MCP tool set, explicit workspace
consent, capability-aware backend selection, independent approval for privileged
operations, and signed evidence. A new supported user should complete a first
verified isolated run in under five minutes without editing JSON.

It does not include transparent replacement of Claude's built-in tools, an
incremental workspace synchronization engine, broad remote MCP support, or a
claim that companion mode isolates Claude itself.

### 11. What does the hosted preview include?

The target preview includes one Agent SDK supervisor per session, a pre-staged
sandbox-owned workspace, typed turn events, durable transcript state, cold
conversation resume, an authenticated Firecracker `vsock` sampling relay,
direct-Anthropic credential binding, Cedar enforcement, bounded OTLP relay,
tenant fencing, patch or artifact export, and native-KVM security evidence.

It excludes general guest internet, guest-initiated clone or push, arbitrary
HTTP tools, remote MCP, provider-native cloud credentials, and stable full-state
resume unless each capability receives a separate design and passes its gates.

### 12. What must Milestone 0 prove before either implementation is approved?

Milestone 0 must produce evidence for four risky contracts:

1. A single long-lived AgentKernel service can own and serialize all bound
   session and sandbox mutations.
2. A Claude Code session can bind to the correct plugin MCP process under
   concurrency without depending on model cooperation.
3. A minimal Agent SDK sampling request from a guest with direct general
   networking disabled can cross an authenticated native-KVM relay with a
   secret canary absent from guest and persisted state.
4. One versioned OpenAPI `AgentSession` schema can represent component health,
   sandboxes, workspace, turns, policy, usage, and receipt references across
   every maintained surface.

It must also complete the hook, local-identity, self-approval, environment
leakage, and plugin-supply-chain threat model and publish a tested compatibility
floor.

### 13. What is the security model, and what if the authoritative service is compromised?

The AgentKernel service is authoritative for bound sessions. For AgentSession
mutations and controlled relays, it authenticates the principal, tenant, client,
session, and sandbox; resolves a backend that meets every hard requirement;
evaluates policy before dispatch and again at the credential proxy; requires
independent approval for privileged work; strips guest-supplied credentials;
records redacted evidence; and fails closed rather than weakening assurance.
Claude's permission system can add a denial but cannot grant AgentKernel
authority. In optional companion mode, unrelated Claude-native host operations
continue outside this boundary. Managed fail-closed behavior applies only to
explicitly covered operations supported by a blocking hook or managed launcher.

The authoritative service is also a high-value trust center. Its compromise
could abuse bound credentials, lifecycle authority, policy decisions, or
evidence for the tenants and sessions it can reach. Before hosted preview, the
child threat model must bound that blast radius through tenant- and
session-scoped credentials, least-privilege components, protected signing keys,
rotation and revocation, tamper-evident audit export, recovery, and an incident
response contract. Exact key custody, hardware-backed storage, and managed
service responsibilities remain open decisions; the design must not pretend the
control plane is outside the threat model.

### 14. What is the commercial hypothesis?

The hypothesis—not a decision in RFC 0002—is that the native companion and
local AgentKernel path remain easy to adopt through the existing MIT-licensed
project, while enterprise value concentrates in managed distribution, policy
administration, independent approval workflows, trust-rooted evidence, remote
operations, runtime governance, conformance, and support. The companion-to-paid
enterprise funnel is unvalidated.

RFC 0002 does not decide whether hosted mode is customer-operated software,
AgentKernel-managed infrastructure, or both. That choice changes key custody,
capacity ownership, data obligations, incident response, and support liability.
Packaging, licensing, price, deployment model, marketplace motion, and hosted
unit economics—including microVM capacity, storage, relay traffic, telemetry,
retention, security conformance, and support—require a separate business
decision before beta positioning is finalized.

### 15. What is defensible if the plugin can be forked?

The plugin is deliberately not the moat. The harder system is the audited
runtime: authoritative session ownership, backend capability conformance,
portable credential and egress control, independent policy enforcement,
receipts and trust roots, lifecycle recovery, cross-surface compatibility,
release operations, design-partner evidence, and support. Strategic value comes
from making those pieces dependable together.

The product must stand on customer value rather than an acquisition thesis.
Strategic-buyer interest is an optional consequence of paid use, trusted
security evidence, reusable runtime architecture, and operational maturity—not
a launch metric or product requirement.

Anthropic can build adjacent runtime or governance features. AgentKernel wins
only if its customer-controlled, provider-aware runtime reaches trusted
multi-backend operations faster and with less integration burden than customers
or a vendor can justify rebuilding. Compatibility with supported Anthropic
surfaces, paid repeat use, and reusable generic infrastructure are required
evidence; acquisition speculation is not.

### 16. How will we distribute it?

The companion uses Claude's supported plugin and marketplace model, with a
pinned source digest and a machine-readable AgentKernel compatibility record.
User and project installation are managed by the AgentKernel CLI. Enterprises
use managed settings and private marketplace policy. Hosted images, binaries,
plugins, and provenance artifacts must be signed before GA.

This does not imply placement in an Anthropic-operated marketplace or an
Anthropic endorsement. Any external distribution agreement is a separate
decision.

### 17. How will we know customers want it?

Demand, operations, and security answer different questions and must not be
collapsed into one score.

**Customer-demand evidence**

| Measure | Applies to | Target |
|---|---|---:|
| Activated design-partner users completing a second AgentKernel-backed Claude session within 30 days | Companion beta cohort | At least 80% |
| Paid enterprise design partners using real repositories | Program GA evidence, reported by mode; one mode cannot borrow another's evidence | 3 |

**Operational SLOs**

| Measure | Applies to | Target |
|---|---|---:|
| Successful plugin installation in the published supported matrix | Companion beta | At least 95% |
| Median install-to-first-verified-run | Companion beta | Under 5 minutes |
| Successful admitted control operations in the supported matrix, excluding explicit policy denials and user cancellations | Each released mode, measured separately | At least 99.5% |

**Hard conformance gates—not tunable metrics**

| Invariant | Required before | Required result |
|---|---|---:|
| Correct session binding in the concurrent-session stress suite | Companion beta | 100% |
| Workspace exposure without recorded exact-path consent | Companion beta | 0 |
| User-authored content overwritten or lost during migration, update, rollback, or uninstall | Companion beta | 0 |
| Cross-tenant access in control, proxy, state, artifact, workspace, telemetry, or deletion tests | Hosted preview | 0 |
| Undeclared direct general guest egress outside approved relays | Hosted preview | 0 |
| Credential-canary findings in the named supported matrix | Hosted preview | 0 |
| Silent assurance downgrades in any supported dispatch or recovery path | Each released mode, separately | 0 |

Repository stars, clones, downloads, or a successful demo do not substitute for
repeat usage on real repositories.

Before companion beta, the child design must set p90 setup and p50/p95
incremental tool-latency SLOs. Before hosted preview, it must set turn-success,
cold-resume, proxy-recovery, latency, cost, and retention SLOs without weakening
the hard gates above.

Measurement uses explicitly enabled operational telemetry or a design-partner
reporting agreement; it does not require prompt or source capture. Before any
metric is used for a release decision, its supported-environment denominator,
activation definition, observation window, retry treatment, and minimum cohort
must be fixed. Targets not yet measured remain launch criteria, not marketing
claims.

### 18. What are the launch no-go conditions?

Any customer-facing beta or preview is a no-go with an unresolved critical or
high-severity security finding.

**Milestone 0 is incomplete** if session binding remains ambiguous, multiple
processes can mutate the same bound session, independent identity cannot be
established, the relay cannot authenticate the workload, or a secret canary
appears in guest or persisted evidence.

**Companion beta is a no-go** if session binding is race-prone, competing owners
can mutate a bound session, installation requires manual JSON editing, workspace
exposure can occur without recorded consent, the requesting MCP identity or
same-user hook can grant its own permission, managed enforcement can silently
fail open, incompatible versions proceed, migrations overwrite user content,
rollback is unproven, or status cannot distinguish host and sandbox work.

**Hosted preview is a no-go** if a bound provider, control-plane, proxy, or
signing credential—including the direct Anthropic key—enters the guest or
prohibited persisted evidence; any relay is unauthenticated; tenant, session,
sandbox, endpoint, provider, model, or limit binding can be bypassed;
independent approval is absent where required; direct general guest network
egress is possible or required outside declared relays; policy lacks real
provider/model enforcement; proxy state cannot recover safely; tenant
boundaries are unproven; or native-KVM evidence is missing. Separately declared
workload secrets follow their own least-privilege and evidence policy and cannot
be confused with AgentKernel's control credentials.

**General availability is also a no-go** with unsigned release artifacts,
untested rollback, contract drift between maintained surfaces, or unpublished
security, data-handling, support, retention, deletion, and recovery limits.

### 19. What are the largest product and execution risks?

| Risk | Response |
|---|---|
| Customers assume the companion isolates Claude itself | Put the boundary in commands, status, receipts, docs, and support language |
| Anthropic changes plugin, hook, MCP, SDK, or authentication behavior | Publish tested version ranges, run a current-version lane, review vendor changes, and refuse incompatible operations |
| Hook or MCP input is mistaken for independent approval | Make same-user signals advisory and enforce approval in the AgentKernel service |
| Hosted scope expands into a generic network and workspace platform | Keep the first workspace pre-staged with direct general networking disabled; require child designs for each relay or sync capability |
| Cloud-provider support is approximated with insecure header injection | Direct Anthropic first; use provider-native brokers only after separate conformance work |
| Backend marketing outruns backend behavior | Resolve tested capabilities and fail instead of silently downgrading |
| Receipts are described as correctness or deterministic replay | Publish the bounded verification, re-execution, and fork meanings |
| Companion adoption does not translate into enterprise use | Recruit paid design partners early and measure repeat governed sessions on real repositories |

### 20. How do support, failure, and rollback avoid trapping customers?

The product treats diagnosis and reversal as primary flows. Health reports
separate plugin, service, policy, backend, workspace, transcript, proxy, and
sandbox components. Asynchronous mutations are idempotent and continue under an
operation ID after client disconnect. Plugin, service, and hosted image roll
back independently. Older binaries ignore newer session stores rather than
downgrading them. Uninstall and rollback retain customer data, and every retained
resource is reported.

Before any customer-facing beta, AgentKernel must name the support owner, supported
environments, escalation channel, compatibility update window, response targets,
and end-of-support policy. Hosted preview additionally needs explicit retention,
deletion, encryption, regional-storage, transcript-ownership, and data-processing
terms.

### 21. How do legacy sessions and loose Claude installations coexist?

During beta, existing `agentkernel session start/list/stop/save/resume/delete`
behavior remains the legacy default when `--mode` is absent. New companion and
hosted sessions require an explicit mode and use the authoritative service.
HTTP `/sessions` continues to mean terminal recordings; the new wire resource
uses `/v1/agent-sessions`.

Existing loose `.claude/` and `.mcp.json` installations receive a merge preview,
and user-authored content is never overwritten. Migration preserves only
bindings it can prove and never invents a Claude conversation ID. Rollback
leaves the versioned new store intact for a newer binary; an older binary ignores
it rather than downgrading records.

### 22. What is the launch and approval sequence?

1. **Milestone 0 — contract, identity, relay, and threat-model spike.** Prove
   session binding, sole-service ownership, a native-KVM credentialless sampling
   request, the secret-canary boundary, and the draft wire contract.
2. **Companion child design and beta decision.** Specify plugin, workspace,
   approval, compatibility, migration, support, and evidence behavior before
   authorizing implementation or customer exposure.
3. **Policy and proxy substrate decision.** Approve the reusable identity,
   provider/model policy, independent approval, and credential-relay foundation.
4. **Hosted child design and preview decision.** Specify supervisor, workspace,
   transcript, direct-Anthropic relay, cold resume, tenant, recovery, data, and
   native-KVM contracts.
5. **Enterprise and backend expansion.** Add each provider broker, relay, and
   backend only through its own conformance evidence and approval boundary.
6. **Partner and GA hardening.** Require current-version compatibility,
   independent security review, signed provenance, tested rollback, published
   limits, support readiness, and paid design-partner evidence.

Companion and hosted mode can stop, narrow, or reach release decisions
independently.

### 23. Which decisions remain open?

| Decision | Required before |
|---|---|
| Supported Claude session-to-MCP binding under concurrent sessions | Companion child design |
| Authenticated local control transport and peer identity | Companion child design |
| First independent approval surface | Companion beta |
| Exact `/v1/agent-sessions` wire schema and stable errors | Both child designs |
| Endpoint-managed settings profile for hosted sampling | Hosted child design |
| Whether cold conversation resume is sufficient while full-state resume remains preview | Hosted GA decision |
| Anthropic review required for marketplace or “validated” language | External launch positioning |
| Commercial packaging, licensing, price, and support tiers | Beta positioning |
| Final product names and release channels | Brand and release review |
| Primary buyer, initial customer profile, and design-partner cohort | Companion child design |
| Customer-operated, managed, or hybrid hosted deployment | Hosted business and child designs |
| Hosted key custody, data terms, unit economics, and support liability | Hosted preview decision |

These decisions belong in RFC child designs and Beads dependencies. This PRFAQ
does not silently resolve them.

### 24. What does approving this PRFAQ authorize?

Approval means the customer promise, target audiences, product separation,
launch sequence, exclusions, and success measures are directionally useful for
Milestone 0. It does not authorize implementation beyond RFC 0002's Milestone 0,
commit a launch date or price, certify a backend, or permit Anthropic partnership
language.

## Copy and claims guardrails

Until the corresponding gates pass, do not use:

- “available now,” “production-ready,” or a ship date;
- “official,” “certified,” “Anthropic-approved,” or “in partnership with
  Anthropic”;
- “replaces Claude Code's sandbox” or “isolates every Claude tool”;
- “hosted Claude Code” for the Agent SDK product;
- “all Claude actions are governed” or any unqualified companion-wide policy
  claim;
- “air-gapped” or unqualified “networkless”;
- “keys never leave the control plane” or “leak-proof”;
- “tamper-proof receipts” or “cryptographic proof the policy was correct”;
- “the API key can never leak” instead of the bounded tested canary claim;
- “hardware isolation” for Docker, Podman, or an unverified backend;
- “works on every backend” or unqualified provider support;
- “enterprise-grade” or “compliant” without named evidence and scope;
- “stable full-state pause, resume, and fork”;
- “deterministic replay”; or
- adoption, reliability, partner, or performance numbers that have not been
  measured on the named release.

No customer quotation belongs in release copy until a named design partner has
used the relevant mode on a real repository and approved the wording.

The reusable boundary line is:

> **Companion mode isolates selected AgentKernel work; hosted mode is designed
> to isolate a supported Agent SDK workload behind declared control-plane
> relays. Each claim applies only to the versions and backends that passed its
> published conformance gates.**
