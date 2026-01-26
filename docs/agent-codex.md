---
layout: default
title: OpenAI Codex
parent: Agents
nav_order: 2
---

# OpenAI Codex

Run OpenAI's Codex CLI in an isolated sandbox.

## Quick Start

```bash
# Create sandbox with Codex pre-installed
agentkernel create codex-dev --config examples/agents/codex/agentkernel.toml

# Start the sandbox
agentkernel start codex-dev

# Run Codex with your API key
agentkernel attach codex-dev -e OPENAI_API_KEY=$OPENAI_API_KEY

# Inside the sandbox:
codex
```

## API Key

Codex requires an OpenAI API key. Get one from [platform.openai.com](https://platform.openai.com/).

```bash
# Interactive session
agentkernel attach codex-dev -e OPENAI_API_KEY=$OPENAI_API_KEY

# One-off command
agentkernel exec codex-dev -e OPENAI_API_KEY=$OPENAI_API_KEY -- \
  codex "Write a hello world function"
```

## Configuration

The example config at `examples/agents/codex/agentkernel.toml`:

```toml
[sandbox]
name = "codex-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "codex"
compatibility_mode = "codex"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Codex needs network for API calls
mount_cwd = true    # Mount project directory
```

## What's Included

The Codex image includes:

- **Node.js 22** - Runtime
- **Codex CLI** - `@openai/codex`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast code search
- **fd** - Fast file finder
- **jq** - JSON processing
