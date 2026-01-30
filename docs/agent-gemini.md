
# Google Gemini CLI

Use Google's Gemini CLI with agentkernel for isolated code execution.

## Plugin Mode (Recommended)

Gemini runs locally, code execution is sandboxed via MCP:

```bash
# Install the MCP config into your project
agentkernel plugin install gemini

# This adds the agentkernel MCP server to .gemini/settings.json
# Gemini will have access to: run_command, create_sandbox, exec_in_sandbox, list_sandboxes, remove_sandbox
```

For global installation:

```bash
agentkernel plugin install gemini --global
```

## Sandbox Mode

Run Gemini CLI itself inside an isolated sandbox:

```bash
# Create sandbox with Gemini pre-installed
agentkernel create gemini-dev --config examples/agents/gemini/agentkernel.toml

# Start the sandbox
agentkernel start gemini-dev

# Run Gemini with your API key
agentkernel attach gemini-dev -e GEMINI_API_KEY=$GEMINI_API_KEY

# Inside the sandbox:
gemini
```

## API Key

Gemini CLI requires a Google AI API key. Get one from [aistudio.google.com](https://aistudio.google.com/).

```bash
# Interactive session
agentkernel attach gemini-dev -e GEMINI_API_KEY=$GEMINI_API_KEY

# One-off command
agentkernel exec gemini-dev -e GEMINI_API_KEY=$GEMINI_API_KEY -- \
  gemini "Explain this code"
```

## Configuration

The example config at `examples/agents/gemini/agentkernel.toml`:

```toml
[sandbox]
name = "gemini-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "gemini"
compatibility_mode = "gemini"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Gemini needs network for API calls
mount_cwd = true    # Mount project directory
```

## What's Included

The Gemini image includes:

- **Node.js 22** - Runtime
- **Gemini CLI** - `@google/gemini-cli`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast code search
- **fd** - Fast file finder
- **jq** - JSON processing
