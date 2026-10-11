---
title: "Sandboxing AI coding agents with AgentKernel"
description: Set up isolated command execution for coding agents, choose tool-level or whole-agent integration, and control project access.
---

# Sandboxing AI coding agents with AgentKernel

AgentKernel runs commands in isolated environments while your coding agent works on a task. You can give an assistant sandbox tools through MCP, or run the agent itself inside a configured sandbox. Those choices expose different parts of the workflow to isolation.

## Choose how to integrate

| Workflow | What runs inside the sandbox | Setup |
|---|---|---|
| Assistant calls sandbox tools | Commands routed through AgentKernel; the assistant and its other tools remain outside | [MCP server](../api/mcp.md) |
| Agent runs inside a sandbox | The configured agent process and its commands | [Agent integrations](../agents/index.md) |
| Application submits execution requests | Commands sent through your integration | [HTTP API](../api/http.md) or [SDKs](../sdks/index.md) |

Installing an MCP server does not automatically redirect an assistant's ordinary shell or file tools. Configure the client workflow to use the sandbox tools for the operations you want isolated.

## Verify execution before sharing a project

After [installing AgentKernel](../getting-started/installation.md) and configuring a backend:

```bash
agentkernel run python3 -c "print('agent sandbox ready')"
```

Expect `agent sandbox ready`. This verifies basic execution; it does not prove a particular isolation backend or access policy. Check the [backend requirements](../config/backends.md) before choosing your boundary.

## Give the task the access it needs

The default moderate profile does not mount the current directory or home directory. A project-dependent test needs its source made available explicitly. For a project configured with a Development Container, AgentKernel can use that definition:

```bash
# From a project containing a Development Container configuration
agentkernel run --auto-devcontainer -- npm test
```

Review that configuration first: it can supply a workspace mount, environment variables, and a post-create command. Writable mounts allow the workload to change host files. See [Development Containers](../features/devcontainers.md) for the supported configuration.

For other projects, use an explicit [sandbox configuration](../config/toml.md) and [security profile](../config/security.md). Check [secrets handling](../features/secrets.md) and backend support before adding model credentials. Passing a real API key into an agent environment makes that key available to the agent.

## Keep the trial small

Run a disposable branch or project copy, inspect the resulting diff, and verify command failures and cleanup before moving everyday work into the sandbox. For repeat sessions, use the [persistent sandbox workflow](../getting-started/quick-start.md#persistent-sandboxes).

**Next:** [Connect an MCP client](../api/mcp.md), browse [agent integrations](../agents/index.md), or [compare sandbox options](../getting-started/choosing-a-sandbox.md).
