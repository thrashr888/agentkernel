# Claude Code

Use [Claude Code](https://claude.ai/code) with agentkernel for isolated code execution.

## Integration Levels

| Level | How It Works | Setup |
|-------|-------------|-------|
| **MCP Tools** | Claude calls agentkernel MCP tools for sandbox control | `agentkernel plugin install claude` |
| **Guest CLI** | Run Claude Code itself inside an agentkernel sandbox | `agentkernel sandbox create` + `agentkernel attach` |

Claude Code has its own [native sandbox](https://code.claude.com/docs/en/sandboxing) using OS-level primitives (Seatbelt on macOS, bubblewrap on Linux). That sandbox cannot be replaced with a custom runtime. The MCP plugin gives Claude access to agentkernel sandboxes as tools it can call.

## Plugin Mode (Recommended)

Claude runs locally, code execution is sandboxed via MCP:

```bash
# Install the plugin into your project
agentkernel plugin install claude

# This creates:
#   .claude/skills/agentkernel/SKILL.md   — teaches Claude when/how to use sandboxes
#   .claude/commands/sandbox.md           — adds /sandbox command
#   .mcp.json                             — registers the agentkernel MCP server
```

Once installed, Claude can use sandboxed execution automatically via the skill, or you can invoke it explicitly:

```
/sandbox python3 -c "print('Hello from sandbox!')"
/sandbox npm test
/sandbox --profile restrictive python3 untrusted.py
```

For global installation (available in all projects):

```bash
agentkernel plugin install claude --global
```

## Run Claude in a sandbox

From a checkout of this repository, create a Docker sandbox with the
[Claude example config](https://github.com/thrashr888/agentkernel/tree/main/examples/agents/claude-code).
Bind only the project directory you intend to share with the guest:

```bash
agentkernel sandbox create claude-dev --backend docker \
  --config examples/agents/claude-code/agentkernel.toml \
  --dir /path/to/your/project
agentkernel sandbox start claude-dev
agentkernel attach claude-dev
```

Inside the guest, run `claude --version`, then `claude`. The example builds a
Docker image with a pinned CLI. The built-in `claude-sandbox` template is a
separate path that installs its pinned CLI during initialization. Neither
path includes account credentials or configures a Rex host.

## Authenticate inside the guest

- **Personal subscription:** For an interactive guest, follow Claude Code's
  sign-in prompt or run `/login`. Sign in to the isolated guest using the
  supported flow rather than copying your host's credential store. A fresh
  guest without retained auth state may need to sign in again.
- **Noninteractive subscription:** Anthropic documents `claude setup-token` and
  `CLAUDE_CODE_OAUTH_TOKEN` for eligible subscriptions when browser login is
  unavailable. This token is a secret and supports model requests, not Claude
  Remote Control or Claude.ai connectors. Bare mode does not read it. Inject
  it only through an approved runtime secret mechanism; this example does not
  provision it.
- **API billing:** Use `ANTHROPIC_API_KEY` if API billing is intended. See
  [AgentKernel's secret delivery options](../features/secrets.md) for the
  backend in use. Never place a token or API key in a Dockerfile, build arg,
  template, repository, or command argument containing the expanded value.

See [Anthropic's authentication guide](https://code.claude.com/docs/en/authentication)
for supported plans and authentication precedence. Check the guest can reach
the documented authentication and API endpoints under its existing network
policy. Installing the CLI does not establish sign-in, credential retention,
or working model requests.

## Review the guest's access

Keep Claude Code's normal permission prompts. The guest can still access its
mounted files and use any credentials or network access you provide. Do not
mount your home directory or copy your whole `.claude` profile into an image.

For a terminal in Rex, use the separately prepared guest and connection steps
in the [Rex guide](../use-cases/rex.md). The published AgentKernel server
container is not a Claude or Rex guest image.
