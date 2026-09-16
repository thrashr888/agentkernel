---
title: "AgentKernel vs Docker Sandboxes and Docker containers"
description: Understand Docker containers, Docker Sandboxes microVMs, and AgentKernel's selectable backends before choosing an AI coding agent sandbox.
---

# AgentKernel vs Docker Sandboxes

Docker Sandboxes and ordinary Docker containers are different products with different isolation models. Docker Sandboxes runs coding agents in microVMs. AgentKernel lets you choose among VM, container, and hosted backends and exposes sandbox operations through a CLI, HTTP API, and MCP server.

## Compare the right boundary

| Option | Isolation boundary | Useful when |
|---|---|---|
| Ordinary Docker container | Linux namespaces and other kernel controls; containers share the Docker host's kernel | You need a familiar container environment and accept that boundary |
| Docker Sandboxes | A microVM with its own Docker daemon, filesystem, and network | You want Docker's agent sandbox workflow, including building containers inside it |
| AgentKernel with Docker | Docker container isolation | You want AgentKernel's interfaces over an existing container backend |
| AgentKernel with Firecracker | A dedicated guest kernel in a microVM on Linux/KVM | You operate a compatible Linux host and want that VM boundary |
| AgentKernel with Apple Containers | VM-backed Linux containers on supported macOS hosts | You want a native Mac backend and meet its requirements |

Docker Desktop's Linux VM is not a separate VM for every ordinary container inside it. Likewise, selecting AgentKernel's Docker backend does not invoke the separate Docker Sandboxes product.

Sources: [Docker Engine security](https://docs.docker.com/engine/security/) and [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/), checked September 15, 2026. AgentKernel requirements are in the [backend reference](../config/backends.md).

## Which should you try?

Choose Docker Sandboxes if its agent workflow and per-sandbox Docker daemon are what you need. Choose AgentKernel if you need selectable backends or want to call sandbox operations from an [MCP client](../api/mcp.md), [HTTP client](../api/http.md), or [SDK](../sdks/index.md).

Neither a VM nor a container protects files and credentials you deliberately share with a workload. Review mounts, network access, and secret handling along with the isolation boundary.

## Try AgentKernel with your existing Docker setup

With Docker running and [AgentKernel installed](../getting-started/installation.md):

```bash
agentkernel run --backend docker --fast=false --image python:3.12-alpine -- python3 -c "print('hello from Docker')"
```

This explicitly selects a Docker container. To evaluate a VM backend, follow its setup instructions and select that backend explicitly. Do not infer the backend from the command's successful output.

For an existing project, reuse the image or Dockerfile where supported and check workspace paths, users, dependencies, and service ports. Docker Compose orchestration and Docker Sandboxes configuration are not interchangeable with `agentkernel.toml`.

**Next:** [Configure sandbox permissions](../config/security.md), [run a coding agent](../use-cases/coding-agents.md), or return to [all comparisons](../getting-started/comparisons.md).
