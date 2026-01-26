---
layout: default
title: Getting Started
nav_order: 3
---

# Getting Started

This guide walks you through your first sandbox and basic workflows.

## Your First Sandbox

The fastest way to run isolated code is the `run` command:

```bash
# Run a Python command
agentkernel run python3 -c "print('Hello from sandbox!')"

# Run a Node.js command
agentkernel run node -e "console.log('Isolated!')"

# Run a shell command
agentkernel run sh -c "whoami && pwd"
```

The `run` command:
1. Creates a temporary sandbox
2. Auto-detects the right container image from your command
3. Executes the command
4. Cleans up automatically

## Persistent Sandboxes

For ongoing work, create a persistent sandbox:

```bash
# Create a sandbox
agentkernel create my-project

# Start it
agentkernel start my-project

# Run commands
agentkernel exec my-project -- ls -la
agentkernel exec my-project -- python3 --version

# Open an interactive shell
agentkernel attach my-project

# Stop when done
agentkernel stop my-project

# Remove when finished
agentkernel remove my-project
```

## Running AI Agents

agentkernel includes pre-configured images for AI coding agents.

### Claude Code

```bash
# Create a sandbox with Claude Code pre-installed
agentkernel create claude-sandbox \
  --config examples/agents/claude-code/agentkernel.toml

# Start and attach with your API key
agentkernel start claude-sandbox
agentkernel attach claude-sandbox -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# Inside the sandbox, run Claude
claude
```

### Running Commands with Environment Variables

Pass environment variables (like API keys) with `-e`:

```bash
# Single variable
agentkernel exec my-sandbox -e API_KEY=secret -- my-command

# Multiple variables
agentkernel exec my-sandbox \
  -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
  -e DEBUG=true \
  -- claude -p "Hello"
```

## Working with Projects

Mount your project directory into the sandbox:

```bash
# Create with project config that mounts current directory
agentkernel create my-project --config agentkernel.toml --dir .

# Your files are available at /workspace inside the sandbox
agentkernel exec my-project -- ls /workspace
```

Example `agentkernel.toml`:

```toml
[sandbox]
name = "my-project"

[security]
mount_cwd = true    # Mount current directory to /workspace
network = true      # Allow network access

[resources]
vcpus = 2
memory_mb = 1024
```

## Listing and Managing Sandboxes

```bash
# List all sandboxes
agentkernel list

# Output:
# NAME                 STATUS     BACKEND
# my-project           running    docker
# claude-sandbox       stopped    docker

# Check a specific sandbox
agentkernel exec my-project -- echo "Still running!"
```

## Next Steps

- [Commands Reference](commands/) - Full CLI documentation
- [Configuration](configuration/agentkernel-toml) - Config file options
- [Security Profiles](configuration/security-profiles) - Control sandbox permissions
- [Running Claude Code](agents/claude-code) - Detailed agent setup
