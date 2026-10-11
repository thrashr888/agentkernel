
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
| `--build` | Build and use the current project's Dockerfile. Conflicts with `--image` and `--fast`. |
| `-p, --profile <PROFILE>` | Security profile: `permissive`, `moderate`, `restrictive` |
| `-k, --keep` | Keep the sandbox after execution (for debugging) |
| `-F, --fast` | Opt into the container pool (default: off) |
| `-c, --config <FILE>` | Path to agentkernel.toml config file |
| `--devcontainer <FILE>` | Path to a JSONC Development Container file |
| `--auto-devcontainer` | Detect `.devcontainer/devcontainer.json` in the project |
| `-B, --backend <BACKEND>` | Backend: `docker`, `podman`, `firecracker`, `apple`, etc. |
| `--template <NAME>` | Use a template (built-in, local, `github:owner/repo/path`, or file) |
| `--ttl <DURATION>` | TTL for kept sandboxes (e.g. `1h`, `30m`, `3d`; default: `1h`) |
| `--branch` | Derive the name from git on the multi-step path. See the lifecycle limitation below. |
| `--no-network` | Disable network access |
| `-P, --publish <PORT>` | Port mapping (e.g. `8080:80`, `3000`). Repeatable. Omit `--fast`. |
| `--ssh` | Enable SSH access to the sandbox |
| `-S, --secret <BINDING>` | Bind a secret to a host via proxy (`KEY:host`, `KEY=value:host`, `KEY:host:header`). Repeatable. |
| `--secret-file <KEY>` | Inject a vault secret as a file inside the sandbox. Repeatable. |
| `--placeholder-secrets` | Use placeholder tokens instead of real values for `--secret-file`. Real values substituted by proxy. |
| `--receipt <FILE>` | Write a signed execution receipt JSON for this run |

## Examples

### Basic usage

```bash
# Auto-detects python image
agentkernel run python3 -c "print('hello')"

# Auto-detects node image
agentkernel run node -e "console.log('hello')"

# Run inline code without mounting host files
agentkernel run python3 -c "print(1 + 1)"
```

### Specify image

```bash
# Use specific Python version
agentkernel run --image python:3.11-alpine python3 --version

# Use Ubuntu
agentkernel run --image ubuntu:24.04 cat /etc/os-release
```

### Build the project image

Bare `run` commands select an image from the command or project files and do not implicitly build an ambient Dockerfile. Request the project build when you want it:

```bash
agentkernel run --build -- npm test
```

An explicit config or template keeps its configured Dockerfile build behavior.

To run using a project's Development Container configuration (including its
workspace mount, environment, and post-create command), use:

```bash
agentkernel run --auto-devcontainer -- npm test
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

For a predictable per-branch lifecycle, create and manage the sandbox explicitly:

```bash
# Prints the generated project-and-branch sandbox name
agentkernel sandbox create --branch --backend docker --image python:3.12-alpine

# Use the printed name (example: myapp-feature-auth)
agentkernel sandbox start myapp-feature-auth
agentkernel exec myapp-feature-auth -- python3 -c "print(1 + 1)"
agentkernel sandbox stop myapp-feature-auth
```

`run --branch` currently reaches name/reuse handling only on the multi-step
path; an ephemeral backend can bypass it. A retained container is stopped after
`run --keep`, and reuse does not automatically restart it. Use explicit lifecycle
commands when you need to control that state.

### Port mapping

```bash
# Run a web server with port mapping
agentkernel run -P 8080:80 python3 -m http.server 80

# Multiple ports
agentkernel run -P 8080:80 -P 3000:3000 node -e '[80,3000].forEach(port => require("http").createServer((req,res) => res.end("hello")).listen(port, "0.0.0.0"))'
```

Note: Port mapping is not compatible with `--fast` mode (container pool). Omit `--fast` when using `-P`/`--publish`. Lowercase `-p` selects the security profile for `run`; `sandbox create` uses lowercase `-p` for publishing ports.

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
| Others | `alpine:3.24` |

Override with `--image` when needed, or pass `--build` to build the current project's Dockerfile first.

## Exit Codes

A successful command returns `0`; command or runtime failures return nonzero.
The CLI does not consistently preserve a guest's numeric exit code across
backends and error paths, and it does not reserve `125` for runtime failures.
Use the reported error and, where supported, the execution receipt's outcome to
inspect the guest failure.
