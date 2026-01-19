---
name: sandbox
description: Execute commands in isolated sandboxes for security. Use when running untrusted code, system commands, or operations that could affect the host system. Automatically detects the right runtime (Python, Node, Rust, Go, Ruby, etc.) from the command.
---

# Sandbox Skill

Execute commands in secure, isolated containers using agentkernel. This provides hardware-level isolation to protect the host system from potentially dangerous operations.

## When to Use This Skill

Use this skill when:

- Running untrusted or generated code
- Executing shell commands that could modify system state
- Testing code in a clean environment
- Running build/test commands that might have side effects
- Executing code from external sources
- Any operation where isolation is beneficial for security

Do NOT use this skill for:

- Simple file reads/writes (use standard tools)
- Git operations (use standard tools)
- Operations that need access to host credentials or SSH keys

## Instructions

### Basic Usage

Run any command in an isolated sandbox:

```bash
agentkernel run <command> [args...]
```

The runtime is auto-detected from the command:

```bash
# Python
agentkernel run python3 script.py
agentkernel run pip install package-name
agentkernel run pytest

# Node.js
agentkernel run node script.js
agentkernel run npm test
agentkernel run npx create-react-app my-app

# Rust
agentkernel run cargo build
agentkernel run cargo test

# Go
agentkernel run go build
agentkernel run go test ./...

# Ruby
agentkernel run ruby script.rb
agentkernel run bundle exec rspec

# And more: PHP, Java, C/C++, Elixir, Lua, Terraform...
```

### Specifying an Image

Override the auto-detected image:

```bash
agentkernel run --image ubuntu:24.04 apt-get update
agentkernel run --image postgres:16-alpine psql --version
```

### Keeping Sandboxes for Debugging

Keep the sandbox after execution:

```bash
agentkernel run --keep npm test
# Later: agentkernel remove run-<id>
```

### Persistent Sandboxes

For longer-running work:

```bash
# Create and start
agentkernel create my-sandbox
agentkernel start my-sandbox

# Execute commands
agentkernel exec my-sandbox npm test
agentkernel exec my-sandbox python -m pytest

# Clean up
agentkernel stop my-sandbox
agentkernel remove my-sandbox
```

## Supported Languages

| Language | Commands | Docker Image |
|----------|----------|--------------|
| Python | `python`, `python3`, `pip`, `poetry`, `uv`, `pytest` | `python:3.12-alpine` |
| Node.js | `node`, `npm`, `npx`, `yarn`, `pnpm`, `bun` | `node:22-alpine` |
| Rust | `cargo`, `rustc` | `rust:1.85-alpine` |
| Go | `go`, `gofmt` | `golang:1.23-alpine` |
| Ruby | `ruby`, `bundle`, `rails`, `rake` | `ruby:3.3-alpine` |
| Java | `java`, `javac`, `mvn`, `gradle` | `eclipse-temurin:21-alpine` |
| PHP | `php`, `composer` | `php:8.3-alpine` |
| C/C++ | `gcc`, `g++`, `make`, `cmake` | `gcc:14-bookworm` |
| .NET | `dotnet` | `mcr.microsoft.com/dotnet/sdk:8.0` |
| Terraform | `terraform` | `hashicorp/terraform:1.10` |
| Lua | `lua`, `luajit` | `nickblah/lua:5.4-alpine` |
| Shell | `bash`, `sh`, `zsh` | `alpine:3.20` |

## Security Notes

- Sandboxes run in isolated containers (Docker or Podman)
- On Linux with KVM, Firecracker microVMs provide hardware-level isolation
- Network access is available by default (can be restricted)
- Host filesystem is NOT mounted by default
- Each sandbox gets a clean environment

## Error Handling

If a command fails, the sandbox is automatically cleaned up. Use `--keep` to preserve it for debugging:

```bash
agentkernel run --keep failing-command
# Inspect: docker exec -it agentkernel-run-<id> sh
# Clean up: agentkernel remove run-<id>
```
