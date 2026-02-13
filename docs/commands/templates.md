
# agentkernel template

Manage sandbox templates. Templates are pre-configured `agentkernel.toml` configs that can be used with `create --template` or `run --template`.

## Subcommands

| Command | Description |
|---------|-------------|
| `template list` | List available templates |
| `template save --from <SANDBOX> <NAME>` | Save a sandbox's config as a template |
| `template add <SPECIFIER>` | Fetch and cache a GitHub template |
| `template remove <NAME>` | Remove a custom template |

## Built-in Templates

agentkernel ships with 23 built-in templates:

### Language Runtimes

| Template | Image | Use Case |
|----------|-------|----------|
| `python` | `python:3.12-alpine` | Python development |
| `python-ml` | `python:3.12` | Python ML/data science |
| `node` | `node:22-alpine` | Node.js development |
| `node-fullstack` | `node:22` | Full-stack Node.js |
| `typescript` | `node:22-alpine` | TypeScript projects |
| `rust` | `rust:1.85-alpine` | Rust development |
| `rust-ci` | `rust:1.85-alpine` | Rust CI (restrictive) |
| `go` | `golang:1.23-alpine` | Go development |
| `ruby` | `ruby:3.3-alpine` | Ruby development |
| `java` | `eclipse-temurin:21-alpine` | Java development |
| `dotnet` | `mcr.microsoft.com/dotnet/sdk:8.0` | .NET development |
| `bash` | `alpine:3.20` | Shell scripting |
| `c` | `gcc:14-bookworm` | C/C++ development |
| `secure` | `alpine:3.20` | Restrictive (no network) |

### AI Agent Sandboxes

| Template | Image | Use Case |
|----------|-------|----------|
| `amp-sandbox` | `node:22-alpine` | Amp (Sourcegraph) agent |
| `claude-sandbox` | `node:22-alpine` | Claude Code agent |
| `codex-sandbox` | `node:22-alpine` | Codex agent |
| `gemini-sandbox` | `python:3.12-alpine` | Gemini CLI agent |
| `opencode-sandbox` | `golang:1.23-alpine` | OpenCode agent |
| `pi-sandbox` | `node:22-alpine` | Pi coding agent |

### Dev Tools

| Template | Image | Ports | Use Case |
|----------|-------|-------|----------|
| `vscode` | `gitpod/openvscode-server` | 3000 | Browser-based VS Code IDE |
| `coder` | `codercom/code-server` | 8080 | Browser-based VS Code IDE |
| `gitea` | `gitea/gitea` | 3000, 2222 | Self-hosted Git service |

## Examples

### List templates

```bash
$ agentkernel template list
NAME                 SOURCE     IMAGE
bash                 built-in   alpine:3.20
c                    built-in   gcc:14-bookworm
claude-sandbox       built-in   python:3.12-alpine
python               built-in   python:3.12-alpine
rust                 built-in   rust:1.85-alpine
my-custom            local      ubuntu:24.04
...
```

### Create from template

```bash
agentkernel create my-sandbox --template python
agentkernel create ci-runner --template rust-ci -B docker
```

### Spin up a browser IDE

```bash
agentkernel create my-ide --template vscode
agentkernel start my-ide
# Open http://localhost:3000
```

### Self-hosted Git server

```bash
agentkernel create my-git --template gitea
agentkernel start my-git
# Open http://localhost:3000
```

### Save a sandbox as a template

```bash
agentkernel template save --from my-sandbox my-template
```

### Add from GitHub

```bash
agentkernel template add github:thrashr888/agentkernel/templates/python.toml
```

### Remove a custom template

```bash
agentkernel template remove my-template
```

## Template File Format

Templates use the standard `agentkernel.toml` format:

```toml
[sandbox]
name = "my-template"
base_image = "python:3.12-alpine"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true
```

## Storage

- Built-in templates are embedded in the binary
- Custom templates are saved to `~/.local/share/agentkernel/templates/`

## See Also

- [create](create.md) - Create a sandbox with `--template`
- [run](run.md) - Run a command with `--template`
- [Configuration](../config/toml.md) - Full config file reference
