# RFC 0001: Product homepage and documentation information architecture

- Status: Draft
- Owner: AgentKernel maintainers
- Beads issue: `agentkernel-fcoc`
- Created: 2026-08-26

## Summary

AgentKernel should have two deliberately different entry points:

1. a product homepage that explains the problem, the boundary AgentKernel provides, and the fastest path to first use; and
2. a documentation section that contains the complete feature, command, backend, security, SDK, and operations material.

The current documentation homepage tries to serve both roles. It is accurate and comprehensive, but its length makes the product difficult to understand before a reader has decided to learn it. This RFC proposes a concise, outcome-led homepage and moves the existing long-form page to `docs/platform-overview.md`, changing only its document title.

The accompanying mock implements the proposal directly in MkDocs so it can be reviewed as a working page rather than as a static composition.

## Motivation

The current homepage opens with the threat model and continues through benchmarks, CLI usage, agent integrations, security profiles, secret injection, every backend category, workflows, SDKs, enterprise policy, comparisons, installation, and a large link directory. Each section is useful documentation, but together they flatten the message hierarchy:

- a new reader must parse implementation detail before understanding the product;
- the primary action is repeated instead of being made obvious once;
- differentiators such as host-held secrets and portable backends compete with lower-level reference material;
- lifecycle terms such as snapshot, pause, resume, and fork do not receive a concise contract at the point where the product is introduced; and
- the visual system remains that of a documentation article rather than a product entrance.

OpenComputer is a useful reference because its homepage sells one coherent outcome and sends detailed behavior to documentation. AgentKernel should adopt that separation while keeping its own position: an open, portable runtime boundary for existing coding agents rather than a managed agent platform.

## Goals

- Explain AgentKernel in one sentence without requiring prior sandbox terminology.
- Make the first useful action visible in the initial viewport.
- Establish the security boundary visually: selected workspace, policy, host-held secrets, isolated execution, and scoped network access.
- Show that AgentKernel works with existing agents and across local, cluster, and hosted backends.
- Distinguish filesystem snapshots from full-state pause and fork without overstating preview support.
- Preserve all current homepage content in the documentation hierarchy.
- Keep the implementation inside the existing MkDocs deployment and avoid a second website stack.
- Meet keyboard, contrast, responsive-layout, reduced-motion, and no-JavaScript baseline requirements.

## Non-goals

- Rewriting every documentation page.
- Introducing an agent orchestration service, hosted control plane, or pricing page.
- Claiming full-state lifecycle support beyond the currently documented Firecracker preview boundary.
- Rebranding the project or introducing a new logo.
- Adding analytics, forms, animation, or a client-side application to the marketing surface.

## Audience and desired decisions

| Audience | Question on arrival | Desired next action |
| --- | --- | --- |
| Individual developer | Can I let an agent work without exposing my laptop? | Install and run one isolated command |
| Agent-tool builder | Can I keep my harness and use this as compute? | Read the CLI, HTTP, MCP, or SDK docs |
| Platform engineer | Can this move from local development to shared infrastructure? | Review backends, policy, and operations |
| Security reviewer | What crosses the isolation boundary? | Review security profiles, secrets, and audit controls |

## Positioning

### Category

Open-source runtime boundary for coding agents.

### Primary promise

Give every coding agent a safe place to work.

### Supporting explanation

AgentKernel runs existing coding agents inside isolated environments, with dedicated kernels where available, host-held secrets, and one interface from a developer laptop to a cluster.

### Differentiators

1. Existing agents, not a proprietary agent loop.
2. Hardware-backed isolation where the platform supports it, with explicit backend capability reporting elsewhere.
3. Credentials can remain outside the guest and be injected only for approved destinations.
4. One lifecycle and policy surface across local, cluster, and hosted execution.
5. Filesystem snapshots and full-machine continuation remain separate, explicit contracts.

The homepage must avoid universal claims that are only true for a subset of backends. For example, “dedicated kernel” is qualified with “where available,” while the backend documentation remains authoritative.

## Information architecture

The top-level navigation becomes:

```text
Home
Docs
  Overview
  Get started
  Features
  Commands
  Configuration
  AI agents
  API
  SDKs
  Operations
Changelog
```

`docs/index.md` becomes the product homepage. The previous file moves to `docs/platform-overview.md`, with only its document title adjusted, and appears as **Docs → Overview**. Existing deep links remain structurally unchanged except for links that intentionally target the old homepage material.

This is a content move, not a deletion. No existing capability description is discarded by the homepage redesign.

## Homepage structure

### 1. Hero

The first viewport contains the category, primary promise, one explanatory paragraph, installation/docs calls to action, and a real-looking CLI example. The terminal demonstrates the product boundary rather than a contrived success metric.

### 2. Problem statement

“Agents need a computer. They should not need yours.” frames the problem in plain language before discussing runtime internals.

### 3. Boundary diagram

A simple host → policy boundary → sandbox diagram explains what remains on the host and what enters the guest. This is the central product visualization because the trust boundary is otherwise difficult to communicate linearly.

### 4. Core capabilities

Three editorial rows cover host-held secrets, existing-agent compatibility, and backend portability. Each row has one deep link; the homepage does not enumerate every flag or provider.

### 5. Lifecycle vocabulary

The homepage gives short, non-overlapping definitions:

- **Snapshot:** filesystem and installed state; restored processes start fresh.
- **Pause:** full guest memory, device, process, and disk state on exactly compatible Firecracker hosts; preview.
- **Fork:** independent child from an immutable full-state checkpoint; preview.

This language is intentionally stricter than a generic “checkpoint and fork” promise. It prevents callers from assuming process continuity when only disk state is preserved.

### 6. Execution targets

One compact band groups local, cluster, hosted, and interface options. Detailed support and capability matrices remain in docs.

### 7. Final action

The page ends with one installation command and two choices: install or explore the platform overview.

## Visual direction

The mock uses an editorial, systems-oriented visual language:

- warm off-white surfaces and near-black technical sections;
- a single safety-lime accent for action and boundary signaling;
- large plain-language typography paired with monospace operational details;
- rules and structured rows instead of a grid of interchangeable feature cards;
- no gradients, decorative animation, stock illustrations, or fabricated customer logos; and
- a responsive layout that preserves the reading order on small screens.

The design is scoped under `.ak-home` so ordinary documentation pages retain MkDocs Material behavior. The only global structural change is the top-level Docs navigation and loading the scoped stylesheet.

## Technical implementation

- Move `docs/index.md` to `docs/platform-overview.md`.
- Add a new HTML-rich `docs/index.md` with no JavaScript dependency.
- Add `docs/stylesheets/home.css` and register it through `extra_css`.
- Enable MkDocs Material navigation tabs and group existing documentation under a top-level Docs entry.
- Use relative site links so local preview and GitHub Pages subpath deployment both work.
- Keep all interactive elements as native links; no custom focus or keyboard behavior is required.

## Accessibility and performance requirements

- Semantic sections and heading order remain valid.
- The architecture graphic includes a text alternative and all information also appears as visible text.
- Focus states are visible in both light and dark schemes.
- Text and action colors must meet WCAG AA contrast.
- Mobile layout must not require horizontal page scrolling; the terminal may scroll internally for literal commands.
- No remote font, image, JavaScript, animation, or tracking dependency is introduced.
- The page must remain understandable with CSS disabled.

## Content governance

Homepage claims should satisfy all of the following:

- they describe a currently shipped capability or are explicitly labeled preview;
- backend-dependent capabilities are qualified and link to the compatibility matrix;
- benchmarks include their environment and source in documentation instead of appearing as context-free homepage proof;
- security claims describe a concrete control rather than using absolute language such as “unbreakable”; and
- new feature detail is added to docs first, then represented on the homepage only when it changes the core product story.

## Rollout and validation

The change can ship through the existing documentation pipeline. Before publication, validation should include:

1. a strict MkDocs build with no unresolved navigation or link warnings;
2. desktop review in both light and dark themes;
3. responsive review around 360 px, 768 px, and 1280 px widths;
4. keyboard-only navigation through every link;
5. a contrast and semantic-heading check; and
6. verification that `platform-overview/` contains the complete former homepage content.

After publication, the primary qualitative test is whether a new reader can answer these questions after the first two sections: what AgentKernel is, why it exists, what it protects, and what to do next.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Marketing surface drifts from implementation | Keep docs authoritative and require qualified, linked claims |
| Existing users cannot find detailed overview material | Place Platform overview first under Docs and link it from both homepage CTAs |
| Custom CSS becomes brittle across Material upgrades | Scope every component under `.ak-home` and avoid template overrides |
| Full-state preview appears generally available | Label pause and fork as preview and link the exact compatibility contract |
| Homepage becomes long again | Treat seven sections as a budget; feature additions replace or consolidate existing material |

## Alternatives considered

### Keep the current homepage and add a short introduction

This preserves every detail in one place but does not solve the mixed product/documentation role or the lack of visual hierarchy.

### Build a separate React or static marketing site

This allows complete visual control but creates another build, dependency, deployment, and navigation surface. The current proposal can validate the information architecture inside MkDocs first.

### Use the repository README as the homepage

The README is optimized for GitHub evaluation and command reference. It should share the core positioning, but it does not replace a navigable website entrance.

## Decision requested

Approve the two-surface model: a concise product homepage at `/` and the complete existing material under `/platform-overview/` within a top-level Docs section. If the mock proves the direction, follow-up copy refinement should happen against this hierarchy rather than by adding more sections to the homepage.
