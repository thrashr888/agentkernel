
# Rust SDK

Official Rust client for agentkernel. Async-first with `tokio`, builder pattern configuration.

- **Crate**: [`agentkernel-sdk`](https://crates.io/crates/agentkernel-sdk)
- **Source**: [`sdk/rust/`](https://github.com/thrashr888/agentkernel/tree/main/sdk/rust)
- **Requires**: Rust 1.70+, `tokio` runtime

## Install

```toml
[dependencies]
agentkernel-sdk = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick Start

```rust
use agentkernel_sdk::AgentKernel;

#[tokio::main]
async fn main() -> agentkernel_sdk::Result<()> {
    let client = AgentKernel::builder().build()?;
    let output = client.run(&["echo", "hello"], None).await?;
    println!("{}", output.output);
    Ok(())
}
```

## Configuration

### Builder Pattern

```rust
use std::time::Duration;
use agentkernel_sdk::AgentKernel;

let client = AgentKernel::builder()
    .base_url("http://localhost:18888")
    .api_key("sk-...")
    .timeout(Duration::from_secs(60))
    .build()?;
```

### Environment Variables

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

```rust
// Picks up env vars automatically
let client = AgentKernel::builder().build()?;
```

## Running Commands

### Basic Execution

```rust
let output = client.run(&["python3", "-c", "print(1 + 1)"], None).await?;
println!("{}", output.output); // "2\n"
```

### With Options

```rust
use agentkernel_sdk::RunOptions;

let opts = RunOptions {
    image: Some("node:22-alpine".into()),
    profile: Some("restrictive".into()),
    fast: Some(false),
};
let output = client.run(&["npm", "test"], Some(opts)).await?;
```

### Streaming Output

Returns a `Stream` of `StreamEvent`:

```rust
use futures::StreamExt;

let mut stream = client.run_stream(&["python3", "script.py"], None).await?;
while let Some(event) = stream.next().await {
    let event = event?;
    match event.event_type.as_str() {
        "output" => print!("{}", event.data["data"]),
        "done" => println!("Exit code: {}", event.data["exit_code"]),
        "error" => eprintln!("Error: {}", event.data["message"]),
        _ => {}
    }
}
```

## Sandbox Management

### Create and Execute

```rust
// Create a sandbox
use agentkernel_sdk::CreateSandboxOptions;

let opts = CreateSandboxOptions {
    image: Some("python:3.12-alpine".into()),
    vcpus: Some(2),
    memory_mb: Some(1024),
    profile: Some("moderate".into()),
};
let sandbox = client.create_sandbox("my-project", Some(opts)).await?;

// Execute commands
let result = client.exec_in_sandbox("my-project", &["pip", "install", "numpy"]).await?;

// Get info
let info = client.get_sandbox("my-project").await?;

// List all
let sandboxes = client.list_sandboxes().await?;

// Remove
client.remove_sandbox("my-project").await?;
```

### Scoped Sandboxes (Recommended)

`with_sandbox` creates a sandbox, passes it to your closure, and removes it when done — even if the closure returns an error:

```rust
client.with_sandbox("test", Some("python:3.12-alpine"), |sb| async move {
    sb.run(&["pip", "install", "numpy"]).await?;
    let result = sb.run(&["python3", "-c", "import numpy; print(numpy.__version__)"]).await?;
    println!("{}", result.output);
    Ok(())
}).await?;
// sandbox auto-removed
```

## File Operations

```rust
// Read a file
let file = client.read_file("my-sandbox", "tmp/hello.txt").await?;
println!("{}", file.content);

// Write a file
client.write_file("my-sandbox", "tmp/hello.txt", "hello world", None).await?;

// Delete a file
client.delete_file("my-sandbox", "tmp/hello.txt").await?;
```

## Batch Execution

```rust
use agentkernel_sdk::BatchCommand;
let results = client.batch_run(vec![
    BatchCommand { command: vec!["echo".into(), "hello".into()] },
]).await?;
```

## Error Handling

```rust
use agentkernel_sdk::{AgentKernel, Error};

let client = AgentKernel::builder().build()?;

match client.run(&["bad-command"], None).await {
    Ok(output) => println!("{}", output.output),
    Err(Error::Auth(msg)) => eprintln!("Auth: {msg}"),
    Err(Error::Validation(msg)) => eprintln!("Validation: {msg}"),
    Err(Error::NotFound(msg)) => eprintln!("Not found: {msg}"),
    Err(Error::Server(msg)) => eprintln!("Server: {msg}"),
    Err(Error::Network(err)) => eprintln!("Network: {err}"),
    Err(Error::Stream(msg)) => eprintln!("Stream: {msg}"),
    Err(e) => eprintln!("Other: {e}"),
}
```

## Types

### `RunOutput`

```rust
pub struct RunOutput {
    pub output: String,
}
```

### `SandboxInfo`

```rust
pub struct SandboxInfo {
    pub name: String,
    pub status: String,
    pub backend: Option<String>,
}
```

### `StreamEvent`

```rust
pub struct StreamEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}
```

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `health()` | `Result<String>` | Health check |
| `run(command, options)` | `Result<RunOutput>` | Run command in temporary sandbox |
| `run_stream(command, options)` | `Result<impl Stream<Item = Result<StreamEvent>>>` | Run with streaming output |
| `list_sandboxes()` | `Result<Vec<SandboxInfo>>` | List all sandboxes |
| `create_sandbox(name, options)` | `Result<SandboxInfo>` | Create a sandbox |
| `get_sandbox(name)` | `Result<SandboxInfo>` | Get sandbox info |
| `remove_sandbox(name)` | `Result<()>` | Remove a sandbox |
| `exec_in_sandbox(name, command)` | `Result<RunOutput>` | Execute in existing sandbox |
| `read_file(name, path)` | `Result<FileReadResponse>` | Read a file from a sandbox |
| `write_file(name, path, content, options)` | `Result<String>` | Write a file to a sandbox |
| `delete_file(name, path)` | `Result<String>` | Delete a file from a sandbox |
| `get_sandbox_logs(name)` | `Result<Vec<LogEntry>>` | Get sandbox audit logs |
| `batch_run(commands)` | `Result<BatchRunResponse>` | Run commands in parallel |
| `with_sandbox(name, image, closure)` | `Result<T>` | Scoped session with auto-cleanup |
