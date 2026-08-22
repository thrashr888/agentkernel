# Agentkernel Guest Agent

Lightweight agent that runs inside microVMs to handle commands from the host.

## Building

Rust 1.89 or newer is required.

### Native (for testing)

```bash
cargo build --release
```

### Static binary for microVMs (Linux)

The agent should be compiled as a static musl binary for portability:

```bash
# Install musl target
rustup target add x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-musl

# Build for x86_64
cargo build --release --target x86_64-unknown-linux-musl

# Build for aarch64 (ARM64)
cargo build --release --target aarch64-unknown-linux-musl
```

The resulting binary will be in:
- `target/x86_64-unknown-linux-musl/release/agent`
- `target/aarch64-unknown-linux-musl/release/agent`

### Using Docker (cross-compile from macOS)

```bash
# Build for x86_64
docker run --rm --platform linux/amd64 -v "$PWD":/app -w /app rust:1.89-alpine3.22 \
  sh -c "apk add --no-cache musl-dev && cargo build --release --locked --target x86_64-unknown-linux-musl"

# Build for aarch64
docker run --rm --platform linux/arm64 -v "$PWD":/app -w /app rust:1.89-alpine3.22 \
  sh -c "apk add --no-cache musl-dev && cargo build --release --locked --target aarch64-unknown-linux-musl"
```

## Protocol

The agent listens on vsock port 52000 and uses a simple JSON-RPC protocol with length-prefixed messages.

### Request format

```json
{
  "id": "unique-request-id",
  "type": "run",
  "command": ["ls", "-la"],
  "cwd": "/app",
  "env": {"KEY": "value"}
}
```

### Response format

```json
{
  "id": "unique-request-id",
  "exit_code": 0,
  "stdout": "...",
  "stderr": ""
}
```

### Request types

- `run`: Execute a command and return output
- `ping`: Health check (returns success response)
- `shutdown`: Graceful shutdown of the agent

## Installation in rootfs

Copy the agent binary to `/usr/bin/agent` in the rootfs and start it from the init script.
