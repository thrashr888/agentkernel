
# How agentkernel Compares

Most sandboxes are **cloud-hosted services** -- you send code to someone else's infrastructure and pay per execution. agentkernel runs **on your machine**. Your code never leaves your network. No API keys, no vendor accounts, no per-execution billing. Just a single binary that gives each sandbox its own virtual machine.

## At a glance

|  | agentkernel | E2B | Daytona | Docker | Gondolin | Cloudflare | Vercel | Modal | Deno |
|--|:-----------:|:---:|:-------:|:------:|:--------:|:----------:|:------:|:-----:|:----:|
| Runs locally | **yes** | -- | -- | **yes** | **yes** | -- | -- | -- | -- |
| VM isolation | **yes** | **yes** | **yes** | **yes** | **yes** | -- | **yes** | -- | **yes** |
| Open source | **yes** | partial | **yes** | -- | **yes** | -- | -- | -- | -- |
| Free / self-hosted | **yes** | -- | -- | **yes** | **yes** | -- | -- | -- | -- |
| CLI tool | **yes** | -- | -- | **yes** | **yes** | -- | **yes** | -- | -- |
| HTTP API | **yes** | **yes** | **yes** | -- | -- | **yes** | **yes** | **yes** | **yes** |
| MCP server | **yes** | -- | -- | -- | -- | -- | -- | -- | -- |
| Secret proxy injection | **yes** | -- | -- | -- | **yes** | -- | -- | -- | -- |
| Multi-backend | **yes** | -- | -- | -- | -- | -- | -- | -- | -- |
| Network policy controls | **yes** | **yes** | -- | -- | **yes** | -- | -- | -- | **yes** |
| Security profiles | **yes** | -- | -- | -- | -- | -- | -- | -- | -- |

### Details

| | agentkernel | E2B | Daytona | Docker | Gondolin | Cloudflare | Vercel | Modal | Deno |
|--|-------------|-----|---------|--------|----------|------------|--------|-------|------|
| **Isolation** | Firecracker microVM | Firecracker microVM | VM | microVM | QEMU microVM | Container | VM | Container | Firecracker microVM |
| **Boot time** | <1&micro;s warm, ~220ms cold | <200ms | <90ms | -- | -- | -- | -- | -- | <200ms |
| **Pricing** | Free | Pay-per-use | Pay-per-use | Free | Free | Pay-per-use | Pay-per-use | Pay-per-use | Pay-per-use |
| **Backends** | Firecracker, Docker, Podman, Apple Containers, Hyperlight | Firecracker | Proprietary | Docker | QEMU (KVM/HVF/TCG) | Proprietary | Proprietary | Proprietary | Firecracker |
| **SDKs** | Python, Rust, Node.js, Go, Swift | Python, JS, Go, Rust | Python, TS, Go | CLI only | TypeScript | TypeScript | TypeScript | Python | TypeScript |
| **Agents** | Claude, Codex, Gemini, OpenCode | Any | Any | Claude, Codex, Gemini | Any | Any | Any | Any | Any |
| **Secrets** | Proxy injection, placeholder tokens, file injection | Env vars | Env vars | Env vars | Placeholder injection, per-host allowlists | -- | -- | -- | -- |
| **Language** | Rust | -- | -- | Go | TypeScript + Zig | -- | -- | -- | -- |

## E2B

[e2b.dev](https://e2b.dev/) is the most established cloud sandbox for AI agents. Firecracker microVMs, sub-200ms boot, strong SDK support across Python, JavaScript, Go, and Rust.

**Use E2B when**: You want a managed cloud service and are building a SaaS product that needs sandboxed execution at scale.

**Use agentkernel when**: You want local execution, no per-execution costs, and multiple backend options (Firecracker, Docker, Apple Containers).

## Gondolin

[gondolin](https://github.com/earendil-works/gondolin) is the closest architectural competitor. QEMU-based microVMs with a TypeScript host control plane and Zig guest agent. Strong security model: placeholder secret injection (secrets never enter VM), per-request network policy hooks via JavaScript, TLS MITM for HTTPS inspection.

**Use Gondolin when**: You want fine-grained per-request network policies with JavaScript hooks. You're in a TypeScript-first ecosystem.

**Use agentkernel when**: You want faster boot times (Firecracker vs QEMU), smaller images (~25MB vs ~200MB), a smaller attack surface (~50K LOC vs ~1M+ LOC), and multiple backend options including Docker fallback on macOS. agentkernel also offers proxy injection, placeholder tokens, and file-based secret injection.

## Daytona

[daytona.io](https://www.daytona.io/) provides cloud sandbox infrastructure with sub-90ms boot times, Git integration, and LSP support. SDKs in Python, TypeScript, and Go.

**Use Daytona when**: You want cloud-hosted sandboxes with Git integration and IDE features.

**Use agentkernel when**: You want local execution, hardware-level VM isolation, and built-in agent compatibility modes.

## Docker Sandboxes

[Docker AI Sandboxes](https://docs.docker.com/ai/sandboxes/) run lightweight microVMs with private Docker daemons. Supports Claude Code, Codex, Gemini, and Docker's own agent (cagent).

**Use Docker when**: You already use Docker Desktop and want sandboxing integrated into that workflow.

**Use agentkernel when**: You want an independent tool with Firecracker microVMs, an HTTP API, MCP server, and alternative backends (Podman, Apple Containers, Hyperlight).

## Cloudflare Sandbox

[Cloudflare Sandbox](https://sandbox.cloudflare.com/) provides container-based sandboxes with a TypeScript SDK, preview URLs, and WebSocket support.

**Use Cloudflare when**: You're building on Cloudflare Workers and want sandboxes close to your edge infrastructure.

**Use agentkernel when**: You want local execution and VM-level isolation instead of container isolation.

## Vercel Sandbox

[Vercel Sandbox](https://vercel.com/docs/vercel-sandbox) provides ephemeral Linux VMs with a TypeScript SDK. Node.js and Python runtimes.

**Use Vercel when**: You're building on Vercel and want integrated observability dashboards.

**Use agentkernel when**: You want local execution with 12+ auto-detected language runtimes.

## Modal Sandboxes

[Modal Sandboxes](https://modal.com/docs/guide/sandboxes) provide container-based environments with a Python SDK. Strong in the ML community with up to 24-hour sessions.

**Use Modal when**: You're already using Modal for ML workloads.

**Use agentkernel when**: You want local execution, VM-level isolation, and a CLI tool (not Python-only).

## Deno Sandboxes

[Deno Sandboxes](https://deno.com/deploy/sandboxes) use Firecracker microVMs with a JavaScript API. Under 200ms boot and network policy controls.

**Use Deno when**: You're in the Deno ecosystem and want managed Firecracker.

**Use agentkernel when**: You want local execution, multi-language auto-detection, and multiple backend options.

## Other notable projects

**[stereOS](https://github.com/papercomputeco/stereos)** -- Purpose-built NixOS-based operating system for AI agents. Produces bootable VM images ("Mixtapes") with stereosd (system daemon) + agentd (agent lifecycle via tmux). Complementary to agentkernel — builds the guest OS, not the host runtime.

**[Browser Use](https://github.com/browser-use/browser-use)** -- Production agent infrastructure (79K stars) using Unikraft micro-VMs. Pioneered "Pattern 2" — isolate the entire agent, not just tools. Agent gets zero secrets, talks to a control plane that proxies all external calls.

**[justbash.dev](https://justbash.dev)** -- Pure TypeScript bash interpreter with in-memory virtual filesystem. ~0ms cold start but no real isolation, no binary execution. Vercel Sandbox API-compatible upgrade path confirms the lightweight→VM isolation spectrum.

**[Fly.io Machines](https://fly.io/machines)** -- Firecracker-based VMs with millisecond boot times. General-purpose compute, usable as a sandbox backend.

**[Rivet Sandbox Agent SDK](https://www.rivet.dev/)** -- Universal API normalizing interactions across multiple sandbox providers (Daytona, E2B, Vercel, Docker). An abstraction layer rather than infrastructure.

## agentkernel's position

agentkernel occupies a specific niche: **local-first, VM-isolated sandboxing for AI coding agents**.

Most alternatives are cloud services — the right choice for SaaS products at scale. But they require sending code to a third party, charge per execution, and add a network dependency.

agentkernel is for developers who want to run AI agents on their own machines with real isolation. On Linux, you get Firecracker microVMs — the same technology behind E2B, Deno Sandboxes, and AWS Lambda. On macOS 26+, Apple Containers provide native VM isolation. On older macOS, Docker and Podman are fallbacks.

The trade-off: you run the infrastructure yourself. For a single developer or small team, that's a feature. For a platform serving thousands of users, a managed cloud service is the better fit.
