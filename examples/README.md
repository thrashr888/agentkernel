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

### Kubernetes
Run sandboxes as Kubernetes Pods on any cluster. Requires `--features kubernetes`.

```bash
agentkernel create k8s-sandbox --backend kubernetes --config examples/kubernetes/agentkernel.toml
agentkernel start k8s-sandbox
agentkernel exec k8s-sandbox -- echo "hello from k8s"
```

**Features**: Pod isolation, NetworkPolicy, warm pool, optional gVisor/Kata RuntimeClass

### Nomad
Run sandboxes as HashiCorp Nomad job allocations. Requires `--features nomad`.

```bash
agentkernel create nomad-sandbox --backend nomad --config examples/nomad/agentkernel.toml
agentkernel start nomad-sandbox
agentkernel exec nomad-sandbox -- echo "hello from nomad"
```

**Features**: Docker/exec/raw_exec drivers, warm pool, Consul/Vault integration

### Remote Daytona
Run a hosted sandbox through the bundled Daytona bridge.

```bash
npm install --prefix scripts
agentkernel sandbox create remote-daytona --backend daytona -c examples/remote-daytona/agentkernel.toml
agentkernel exec remote-daytona -- sh -lc 'node -v'
```

**Features**: managed `/workspace` sync, resolved preview endpoints, live snapshot/restore

### Remote Runloop
Run a hosted sandbox through the bundled Runloop bridge.

```bash
npm install --prefix scripts
agentkernel sandbox create remote-runloop --backend runloop -c examples/remote-runloop/agentkernel.toml
agentkernel exec remote-runloop -- sh -lc 'pwd && ls -la /workspace'
```

**Features**: managed `/workspace` sync, tunnel-backed endpoints, interactive attach, live snapshot/restore

### Remote E2B
Run a hosted sandbox through the bundled E2B bridge.

```bash
npm install --prefix scripts
agentkernel sandbox create remote-e2b --backend e2b -c examples/remote-e2b/agentkernel.toml
agentkernel exec remote-e2b -- sh -lc 'python3 --version || node -v'
```

**Features**: managed `/workspace` sync, file APIs, PTY attach, live snapshot/restore

### Remote Modal
Run a hosted sandbox through the bundled Modal bridge.

```bash
npm install --prefix scripts
agentkernel sandbox create remote-modal --backend modal -c examples/remote-modal/agentkernel.toml
agentkernel exec remote-modal -- sh -lc 'pwd && ls -la /workspace'
```

**Features**: managed `/workspace` sync, tunnel-backed endpoints, interactive attach, live snapshot/restore

### Enterprise Policies
Centralized Cedar policy management with RBAC, MFA enforcement, and runtime restrictions. Requires `--features enterprise`.

```bash
agentkernel create enterprise-sandbox --config examples/enterprise/agentkernel.toml
```

**Features**: Cedar policies, Ed25519 signed bundles, RBAC, MFA gates, org isolation, offline caching

### Error App (Expected to Fail)
An example that uses a non-existent image to test error handling.

```bash
agentkernel create error-app --config examples/error-app/agentkernel.toml
agentkernel start error-app  # This will fail as expected
```

**Purpose**: Validates that agentkernel handles invalid configurations gracefully

### Durable Stores
Create payload templates for durable store resources:

- `examples/durable-stores/sqlite-store.json`
- `examples/durable-stores/postgres-store.json`
- `examples/durable-stores/mysql-store.json`
- `examples/durable-stores/redis-store.json`

See `examples/durable-stores/README.md` for `curl` examples.

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
