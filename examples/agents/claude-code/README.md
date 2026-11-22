# Claude Code Agent Image

A Docker image with Claude Code CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel sandbox create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel sandbox start my-project
agentkernel attach my-project

# Inside the sandbox, run Claude Code
claude
```

The Dockerfile is built automatically when you use the config file.

## Authentication

The image includes the stock Claude Code CLI, not credentials. Run
`claude --version` before signing in. For an interactive session, use the
Claude Code sign-in flow inside the guest. For a noninteractive session,
Anthropic supports an eligible subscription token or an API key, depending on
the mode and billing you intend. See the
[authentication guide](https://code.claude.com/docs/en/authentication).

Pass credentials only through an approved runtime mechanism. Do not bake them
into images or templates, pass expanded values as command arguments, or copy
your host's auth profile. This example does not verify subscription login,
token refresh, or persistence across guest replacement.

## What's Included

- **Node.js 24 LTS** - Runtime for Claude Code
- **Claude Code CLI** - `@anthropic-ai/claude-code`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search (used by Claude Code)
- **fd** - Fast file finder
- **jq** - JSON processing
- **bash** - Shell

## Customizing

Create your own Dockerfile based on this one:

```dockerfile
FROM agentkernel/claude-code
USER root

# Add your tools
RUN apk add --no-cache your-tools

# Add project-specific setup
# Copy only non-secret, project-specific settings
COPY --chown=developer:developer your-config /home/developer/.config/
USER developer
```

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Guest network access and mounted files remain subject to backend and runtime policy
- This image is not the AgentKernel server image or a prepared Rex guest
