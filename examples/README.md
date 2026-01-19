# Agentkernel Examples

Example configurations for running AI agents in sandboxed environments.

## Examples

### Python App
A Flask web application demonstrating Python sandbox configuration.

```bash
agentkernel create python-app --config examples/python-app/agentkernel.toml
agentkernel start python-app
```

### Go App
A simple Go HTTP server demonstrating Go sandbox configuration.

```bash
agentkernel create go-app --config examples/go-app/agentkernel.toml
agentkernel start go-app
```

### Rust App
A Rust TCP server demonstrating Rust sandbox configuration.

```bash
agentkernel create rust-app --config examples/rust-app/agentkernel.toml
agentkernel start rust-app
```

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
