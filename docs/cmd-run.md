---
title: run
permalink: /cmd-run.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# agentkernel run

Run a command in a temporary sandbox. The sandbox is created, the command executed, and then cleaned up automatically.

## Usage

```bash
agentkernel run [OPTIONS] <COMMAND>...
```

## Options

| Option | Description |
|--------|-------------|
| `--image <IMAGE>` | Docker image to use (auto-detected if not specified) |
| `--profile <PROFILE>` | Security profile: `permissive`, `moderate`, `restrictive` |
| `--keep` | Keep the sandbox after execution (for debugging) |
| `--fast` | Use container pool for faster startup (default: true) |
| `--config <FILE>` | Path to agentkernel.toml config file |

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
agentkernel list
agentkernel exec <sandbox-name> -- cat /tmp/debug.log
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
