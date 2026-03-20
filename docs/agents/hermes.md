
# Hermes Agent

Run [Hermes Agent](https://github.com/NousResearch/hermes-agent) (NousResearch) with agentkernel for isolated code execution.

## Quick Start

Hermes Agent is a self-improving autonomous AI agent with 40+ built-in tools including terminal, file operations, web search, browser automation, vision, and code execution. It features persistent memory and a skills system that learns from experience.

```bash
# Option 1: Use the built-in template
agentkernel run --template hermes-sandbox -- python -m hermes_cli.main --help

# Option 2: Use the Dockerfile example
agentkernel sandbox create hermes-dev --config examples/agents/hermes/agentkernel.toml
agentkernel sandbox start hermes-dev
agentkernel attach hermes-dev
```

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/tap && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Create and start a sandbox

```bash
agentkernel sandbox create hermes-dev --config examples/agents/hermes/agentkernel.toml
agentkernel sandbox start hermes-dev
```

### 3. Launch Hermes

```bash
agentkernel attach hermes-dev
# Inside the sandbox:
cd /opt/hermes-agent && python -m hermes_cli.main
```

## API Keys

Hermes supports multiple LLM providers via LiteLLM. Pass at least one provider key:

```bash
# OpenRouter (recommended - access to all models)
export OPENROUTER_API_KEY=sk-or-...

# Anthropic (direct)
export ANTHROPIC_API_KEY=sk-ant-...

# OpenAI (direct)
export OPENAI_API_KEY=sk-...
```

## Configuration

Hermes uses two config files inside `~/.hermes/`:

- **`.env`** - API keys and backend settings
- **`config.yaml`** - Model selection, tools, memory, compression settings

Key config options in `config.yaml`:

```yaml
model:
  default: "anthropic/claude-opus-4.6"
  provider: "auto"

terminal:
  backend: "local"
  timeout: 180

toolsets:
  - all

memory:
  memory_enabled: true
```

Run `hermes setup` inside the sandbox for a guided configuration wizard.

The example config at `examples/agents/hermes/agentkernel.toml`:

```toml
[sandbox]
name = "hermes-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "hermes"

[resources]
vcpus = 2
memory_mb = 2048

[security]
profile = "moderate"
network = true
mount_cwd = true
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENROUTER_API_KEY` | - | OpenRouter API key (recommended) |
| `ANTHROPIC_API_KEY` | - | Anthropic API key |
| `OPENAI_API_KEY` | - | OpenAI API key |
| `HERMES_MODEL` | - | Override default model |
| `HERMES_HOME` | `~/.hermes` | Config directory |

## What's Included

The sandbox image includes:

- **Python 3.11** - Runtime for Hermes
- **Node.js 22** - For browser automation tools
- **Hermes Agent** - Full install with 40+ tools
- **mini-swe-agent** - Terminal tool backend
- **Git** - Version control
- **ripgrep** - Fast code search
- **jq** - JSON processing

## Customizing

Create a custom Dockerfile based on the example:

```dockerfile
FROM nikolaik/python-nodejs:python3.11-nodejs22

# Base tools
RUN apt-get update && apt-get install -y --no-install-recommends git bash ripgrep jq

# Hermes Agent
RUN git clone --depth 1 https://github.com/NousResearch/hermes-agent.git /opt/hermes-agent \
    && cd /opt/hermes-agent \
    && git submodule update --init --depth 1 \
    && pip install -e ".[all]" \
    && pip install -e "./mini-swe-agent" \
    && npm install

# Your additions
RUN pip install your-custom-packages

WORKDIR /workspace
```
