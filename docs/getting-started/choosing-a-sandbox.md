---
title: "Choosing the best sandbox for AI coding agents"
description: Choose an AI agent sandbox by execution location, isolation boundary, integration, state requirements, and a repeatable workload trial.
---

# Choosing the best sandbox for AI coding agents

The best sandbox is the one that meets your workload's execution, permission, and operating requirements. Start with where code may run and what it must access, then test the integration with a real task.

## Start with your requirements

| Your requirement | Start here | Verify before adopting |
|---|---|---|
| Run commands on your own workstation | [AgentKernel quickstart](quick-start.md) | Host support, backend selection, and which files are shared |
| Operate a Linux microVM backend | [Firecracker setup](installation.md#linux-firecracker) | KVM access, guest dependencies, and service lifecycle |
| Run on a supported Mac | [Backend options](../config/backends.md) | Apple Containers requirements or the container boundary of Docker/Podman |
| Use a managed sandbox service | [E2B comparison](../comparisons/e2b.md) or [Daytona comparison](../comparisons/daytona.md) | Deployment options, data location, credentials, and provider limits |
| Build containers inside an agent sandbox | [Docker Sandboxes comparison](../comparisons/docker.md) | Whether the selected product provides a private Docker daemon |
| Let an assistant invoke sandbox tools | [MCP setup](../api/mcp.md) | Client configuration, tool permissions, and actual execution backend |
| Preserve running processes across pause and resume | [Firecracker full-state preview](../operations/firecracker-full-state.md) | Exact host compatibility and native validation; filesystem snapshots are different |

These are starting points for evaluation, not a ranking based on a shared benchmark.

## Run one representative task

Use the same input, dependencies, and expected output for each candidate. A useful trial includes a successful run, a command failure, a timeout, and cleanup. If the workload edits files, confirm which edits persist and which reach your host.

Measure the full operation your user waits for: provisioning, image preparation, file transfer, command execution, and cleanup. Report cold and warm runs separately. A pre-warmed pool lookup cannot be compared directly with a fresh VM boot. AgentKernel's [benchmarks](benchmarks.md) document its existing measurements; they do not establish a universal provider ranking.

## Decide what the agent may access

A local sandbox can still make network requests. A cloud model can receive prompts from a locally running agent. Document both execution location and model access if keeping code within a boundary matters to your project.

Start with [security profiles](../config/security.md), then review backend support for each required control. Share only the repository and credentials the task needs. A mounted working directory is writable host data when mounted with write access.

## Account for operating work

Local execution uses your hardware and requires updates, disk management, and troubleshooting. Hosted execution introduces provider credentials, service limits, and charges. Compare total operating effort for your workload rather than assuming either option is free to operate.

**Try it:** [Install AgentKernel](installation.md), run the [quickstart](quick-start.md), and follow the [coding agent workflow](../use-cases/coding-agents.md). For provider-specific trade-offs, see [all comparisons](comparisons.md).
