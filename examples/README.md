# Agentkernel Examples

Example configurations for running AI agents in sandboxed environments.

## Quick Start

Run all examples:
```bash
./scripts/run-examples.sh
```

Run a specific example:
```bash
./scripts/run-examples.sh python-app
```

List available examples:
```bash
./scripts/run-examples.sh --list
```

## Examples

### Python App
A Flask web application demonstrating Python sandbox configuration.

```bash
agentkernel create python-app --config examples/python-app/agentkernel.toml
agentkernel start python-app
agentkernel exec python-app python3 --version
```

**Features**: Flask, pytest, ruff linting, uv package manager

### Node.js App
A Node.js HTTP server demonstrating JavaScript sandbox configuration.

```bash
agentkernel create node-app --config examples/node-app/agentkernel.toml
agentkernel start node-app
agentkernel exec node-app node --version
```

**Features**: Node 22, npm, Express-ready

### TypeScript App
A TypeScript HTTP server demonstrating TypeScript sandbox configuration.

```bash
agentkernel create typescript-app --config examples/typescript-app/agentkernel.toml
agentkernel start typescript-app
agentkernel exec typescript-app npx tsx server.ts
```

**Features**: TypeScript, tsx for execution, type checking

### Go App
A simple Go HTTP server demonstrating Go sandbox configuration.

```bash
agentkernel create go-app --config examples/go-app/agentkernel.toml
agentkernel start go-app
agentkernel exec go-app go version
```

**Features**: Go 1.23, go modules, make

### Rust App
A Rust TCP server demonstrating Rust sandbox configuration.

```bash
agentkernel create rust-app --config examples/rust-app/agentkernel.toml
agentkernel start rust-app
agentkernel exec rust-app rustc --version
```

**Features**: Rust 1.85, cargo, clippy, rustfmt

### Ruby App
A Ruby HTTP server demonstrating Ruby sandbox configuration.

```bash
agentkernel create ruby-app --config examples/ruby-app/agentkernel.toml
agentkernel start ruby-app
agentkernel exec ruby-app ruby --version
```

**Features**: Ruby 3.3, Bundler, RSpec, RuboCop

### Java App
A Java HTTP server demonstrating Java sandbox configuration.

```bash
agentkernel create java-app --config examples/java-app/agentkernel.toml
agentkernel start java-app
agentkernel exec java-app java --version
```

**Features**: Java 21 (Eclipse Temurin), javac

### C/C++ App
A C and C++ HTTP server demonstrating native compilation.

```bash
agentkernel create c-app --config examples/c-app/agentkernel.toml
agentkernel start c-app
agentkernel exec c-app gcc --version
```

**Features**: GCC 14, G++, make, Debian Bookworm base

### .NET/C# App
A C# HTTP server demonstrating .NET sandbox configuration.

```bash
agentkernel create dotnet-app --config examples/dotnet-app/agentkernel.toml
agentkernel start dotnet-app
agentkernel exec dotnet-app dotnet --version
```

**Features**: .NET 8 SDK, C#, F#

### Bash/Shell App
A minimal shell script example using Alpine Linux.

```bash
agentkernel create bash-app --config examples/bash-app/agentkernel.toml
agentkernel start bash-app
agentkernel exec bash-app echo "Hello!"
```

**Features**: Alpine 3.20, busybox, shell scripting

### Error App (Expected to Fail)
An example that uses a non-existent image to test error handling.

```bash
agentkernel create error-app --config examples/error-app/agentkernel.toml
agentkernel start error-app  # This will fail as expected
```

**Purpose**: Validates that agentkernel handles invalid configurations gracefully

## Configuration Schema

Each `agentkernel.toml` defines:

- **sandbox**: Name and base Docker image
- **agent**: Preferred AI coding agent (claude, gemini, codex, opencode)
- **environment**: Language-specific settings
- **dependencies**: System and language packages to install
- **scripts**: Common tasks (setup, test, lint, build, run)
- **mounts**: Directory mappings into the sandbox
- **network**: Exposed ports
- **cache**: Optional caching for faster rebuilds

## Benchmarking

Run benchmarks to measure sandbox operation performance:

```bash
./scripts/benchmark.sh        # Run 5 iterations (default)
./scripts/benchmark.sh 10     # Run 10 iterations
```

This measures create, start, exec, stop, and remove times.
