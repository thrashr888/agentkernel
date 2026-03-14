
# Symphony

Run [OpenAI Symphony](https://github.com/openai/symphony) with agentkernel for isolated orchestration of autonomous coding agents.

## Quick Start

Symphony is an orchestration daemon that monitors issue trackers (Linear), spawns autonomous coding agents (Codex), and manages the full lifecycle of implementation runs. It turns project management boards into autonomous coding pipelines.

```bash
# Option 1: Use the built-in template
agentkernel run --template symphony-sandbox -- elixir --version

# Option 2: Use the Dockerfile example
agentkernel sandbox create symphony-dev --config examples/agents/symphony/agentkernel.toml
agentkernel sandbox start symphony-dev
agentkernel attach symphony-dev
```

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Create and start a sandbox

```bash
agentkernel sandbox create symphony-dev --config examples/agents/symphony/agentkernel.toml
agentkernel sandbox start symphony-dev
```

### 3. Configure and launch Symphony

```bash
agentkernel attach symphony-dev

# Inside the sandbox, create a WORKFLOW.md:
cat > /workspace/WORKFLOW.md << 'WORKFLOW'
---
tracker:
  kind: linear
  project_slug: "YOUR-PROJECT-SLUG"
  api_key: "$LINEAR_API_KEY"
  active_states: ["Todo", "In Progress"]
  terminal_states: ["Closed", "Cancelled", "Done"]

polling:
  interval_ms: 30000

workspace:
  root: /workspace/symphony-workspaces

hooks:
  after_create: |
    git clone --depth 1 git@github.com:your-org/your-repo.git .

agent:
  max_concurrent_agents: 5
  max_turns: 20

codex:
  command: codex app-server
  approval_policy: never
  thread_sandbox: workspace-write
---

You are working on issue {{ issue.identifier }}: {{ issue.title }}

{{ issue.description }}
WORKFLOW

# Run Symphony with dashboard
cd /opt/symphony/elixir && mix run -- /workspace/WORKFLOW.md --port 4000
```

## API Keys

Symphony requires two API keys:

```bash
# Linear (for issue tracking)
export LINEAR_API_KEY=lin_api_...

# OpenAI (for Codex agent)
export OPENAI_API_KEY=sk-...
```

## Configuration

Symphony uses a single **`WORKFLOW.md`** file with YAML front matter and a Liquid-templated Markdown prompt body.

Key configuration sections:

| Section | Purpose |
|---------|---------|
| `tracker` | Linear project connection and state mapping |
| `polling` | How often to check for new issues |
| `workspace` | Where to create per-issue workspaces |
| `hooks` | Shell scripts for workspace setup (e.g., git clone) |
| `agent` | Concurrency limits and turn limits |
| `codex` | Codex CLI command, approval policy, sandbox mode |

The example config at `examples/agents/symphony/agentkernel.toml`:

```toml
[sandbox]
name = "symphony-sandbox"

[build]
dockerfile = "Dockerfile"

[resources]
vcpus = 2
memory_mb = 2048

[security]
profile = "moderate"
network = true
mount_cwd = true
```

## HTTP API

When started with `--port`, Symphony exposes a dashboard and API:

| Endpoint | Description |
|----------|-------------|
| `GET /` | Live dashboard |
| `GET /api/v1/state` | JSON system state |
| `GET /api/v1/<issue>` | Per-issue status |
| `POST /api/v1/refresh` | Force immediate poll |

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `LINEAR_API_KEY` | Yes | Linear API key |
| `OPENAI_API_KEY` | Yes | OpenAI API key for Codex |
| `SYMPHONY_WORKSPACE_ROOT` | No | Override workspace directory |
| `CODEX_BIN` | No | Custom path to Codex executable |

## What's Included

The sandbox image includes:

- **Elixir 1.19** with **OTP 28** - Runtime for Symphony
- **Node.js 22** - For Codex CLI
- **Codex CLI** - `@openai/codex` (coding agent)
- **Git** - Version control
- **bash** - Shell

## Customizing

Create a custom Dockerfile based on the example:

```dockerfile
FROM elixir:1.19-otp-28-slim

# Base tools + Node.js
RUN apt-get update && apt-get install -y --no-install-recommends \
    git bash ca-certificates gnupg curl \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Codex CLI
RUN npm install -g @openai/codex

# Symphony
RUN mix local.hex --force && mix local.rebar --force
RUN git clone --depth 1 https://github.com/openai/symphony.git /opt/symphony \
    && cd /opt/symphony/elixir && mix deps.get && mix compile

# Your additions
RUN npm install -g your-custom-tools

WORKDIR /workspace
```
