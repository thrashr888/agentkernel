
# agentkernel run

Run a command in a temporary sandbox. The sandbox is created, the command executed, and then cleaned up automatically.

## Usage

```bash
agentkernel run [OPTIONS] <COMMAND>...
```

## Options

| Option | Description |
|--------|-------------|
| `-i, --image <IMAGE>` | Docker image to use (auto-detected if not specified) |
| `-p, --profile <PROFILE>` | Security profile: `permissive`, `moderate`, `restrictive` |
| `-k, --keep` | Keep the sandbox after execution (for debugging) |
| `-F, --fast` | Use container pool for faster startup (default: true) |
| `-c, --config <FILE>` | Path to agentkernel.toml config file |
| `-B, --backend <BACKEND>` | Backend: `docker`, `podman`, `firecracker`, `apple`, etc. |
| `--template <NAME>` | Use a template (built-in, local, `github:owner/repo/path`, or file) |
| `--ttl <DURATION>` | TTL for kept sandboxes (e.g. `1h`, `30m`, `3d`; default: `1h`) |
| `--branch` | Use git project+branch as sandbox name (reuses existing sandbox) |
| `--no-network` | Disable network access |
| `-P, --publish <PORT>` | Port mapping (e.g. `8080:80`, `3000`). Repeatable. Requires `--fast=false`. |
| `--receipt <FILE>` | Write a signed execution receipt JSON for this run |

## Examples

### Basic usage

```bash
# Auto-detects python image
agentkernel run python3 -c "print('hello')"

# Auto-detects node image
agentkernel run node -e "console.log('hello')"

# Run a script
agentkernel run python3 script.py
```

### Specify image

```bash
# Use specific Python version
agentkernel run --image python:3.11-alpine python3 --version

# Use Ubuntu
agentkernel run --image ubuntu:24.04 cat /etc/os-release
```

### Security profiles

```bash
# Restrictive: no network, read-only filesystem
agentkernel run --profile restrictive python3 -c "print('isolated')"

# Permissive: full network, mount home directory
agentkernel run --profile permissive curl https://api.example.com
```

### Keep sandbox for debugging

```bash
# Sandbox persists after command exits
agentkernel run --keep python3 script.py

# Later, inspect the sandbox
agentkernel sandbox list
agentkernel exec <sandbox-name> -- cat /tmp/debug.log
```

### Branch-aware execution

```bash
# Reuses sandbox named after your git project + branch
# On branch "feature/auth" in project "myapp" → sandbox "myapp-feature-auth"
agentkernel run --branch -- npm test

# Subsequent runs reuse the same sandbox (faster, state preserved)
agentkernel run --branch -- npm run lint
```

### Port mapping

```bash
# Run a web server with port mapping (requires --fast=false)
agentkernel run -p 8080:80 --fast=false python3 -m http.server 80

# Multiple ports
agentkernel run -p 8080:80 -p 3000:3000 --fast=false node server.js
```

Note: Port mapping is not compatible with `--fast` mode (container pool). Use `--fast=false` or omit `--fast` when using `-p`.

### From a template

```bash
agentkernel run --template python -- python3 -c "print('hello')"
agentkernel run --template rust-ci -- cargo test
```

### Execution receipt

```bash
# Run and emit a receipt
agentkernel run --receipt ./run-receipt.json -- python3 -c "print('ok')"

# Verify receipt integrity
agentkernel receipt verify ./run-receipt.json

# Replay the recorded invocation and compare hash/exit code
agentkernel receipt replay ./run-receipt.json
```

## Auto-Detection

The `run` command automatically selects an appropriate Docker image based on your command:

| Command starts with | Image selected |
|---------------------|----------------|
| `python3`, `python`, `pip` | `python:3.12-alpine` |
| `node`, `npm`, `npx`, `yarn` | `node:22-alpine` |
| `cargo`, `rustc` | `rust:1.85-alpine` |
| `go` | `golang:1.23-alpine` |
| `ruby`, `gem`, `bundle` | `ruby:3.3-alpine` |
| Others | `alpine:3.20` |

Override with `--image` when needed.

## Exit Codes

The command returns the exit code from the executed command, or:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Command failed |
| 125 | agentkernel error (sandbox creation failed, etc.) |
