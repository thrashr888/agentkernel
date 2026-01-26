---
title: Quick Start
permalink: /getting-started.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# Quick Start

This guide walks you through your first sandbox.

## One-Shot Commands

Run a command in a temporary, isolated sandbox:

```bash
# Python
agentkernel run python3 -c "print('Hello from sandbox!')"

# Node.js
agentkernel run node -e "console.log('Hello from sandbox!')"

# Shell
agentkernel run sh -c "uname -a"
```

The sandbox is automatically created, the command runs, and cleanup happens.

## Persistent Sandboxes

For longer sessions, create a named sandbox:

```bash
# Create a sandbox
agentkernel create my-sandbox --image python:3.12-alpine

# Start it
agentkernel start my-sandbox

# Run commands
agentkernel exec my-sandbox -- python3 --version
agentkernel exec my-sandbox -- pip install requests

# Attach for interactive shell
agentkernel attach my-sandbox

# Stop when done
agentkernel stop my-sandbox

# Remove the sandbox
agentkernel remove my-sandbox
```

## Using Config Files

For complex setups, use `agentkernel.toml`:

```toml
[sandbox]
name = "dev"

[build]
dockerfile = "Dockerfile"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true
```

Then reference it:

```bash
agentkernel create dev --config agentkernel.toml
```

## Running AI Agents

See the [Agents](agents.html) section for running Claude Code, Codex, and Gemini in sandboxes.

```bash
# Quick example with Claude Code
agentkernel create claude-sandbox --config examples/agents/claude-code/agentkernel.toml
agentkernel start claude-sandbox
agentkernel attach claude-sandbox -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
```

## Next Steps

- [Commands Reference](commands.html) - Full CLI documentation
- [Configuration](configuration.html) - Config file format
- [Security Profiles](config-security.html) - Permission presets
