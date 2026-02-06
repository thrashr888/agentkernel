# agentkernel-sdk

Rust SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run AI coding agents in secure, isolated microVMs.

## Install

```toml
[dependencies]
agentkernel-sdk = "0.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick Start

```rust,no_run
use agentkernel_sdk::AgentKernel;

#[tokio::main]
async fn main() -> agentkernel_sdk::Result<()> {
    let client = AgentKernel::builder().build()?;
    let output = client.run(&["echo", "hello"], None).await?;
    println!("{}", output.output);
    Ok(())
}
```

## Sandbox Sessions

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
use agentkernel_sdk::{AgentKernel, CreateSandboxOptions};

let client = AgentKernel::builder().build()?;
let opts = CreateSandboxOptions {
    image: Some("python:3.12-alpine".to_string()),
    ..Default::default()
};

client.with_sandbox("test", Some(opts), |sb| async move {
    sb.run(&["pip", "install", "numpy"]).await?;
    let result = sb.run(&["python3", "-c", "import numpy; print(numpy.__version__)"]).await?;
    println!("{}", result.output);
    Ok(())
}).await?;
// sandbox auto-removed
# Ok(())
# }
```

## Exec Options

Run commands with a working directory, environment variables, or as root:

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
# let client = agentkernel_sdk::AgentKernel::builder().build()?;
use agentkernel_sdk::ExecOptions;

let opts = ExecOptions {
    workdir: Some("/app".to_string()),
    env: vec!["NODE_ENV=production".to_string()],
    sudo: Some(true),
    ..Default::default()
};
let output = client.exec_in_sandbox("my-sandbox", &["npm", "start"], Some(opts)).await?;
# Ok(())
# }
```

## Git Source Cloning

Clone a git repo into the sandbox at creation time:

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
# let client = agentkernel_sdk::AgentKernel::builder().build()?;
use agentkernel_sdk::CreateSandboxOptions;

let opts = CreateSandboxOptions {
    image: Some("node:20-alpine".to_string()),
    source_url: Some("https://github.com/user/repo.git".to_string()),
    source_ref: Some("main".to_string()),
    ..Default::default()
};
client.create_sandbox("my-project", Some(opts)).await?;
# Ok(())
# }
```

## Persistent Volumes

Mount volumes that persist across sandbox restarts:

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
# let client = agentkernel_sdk::AgentKernel::builder().build()?;
use agentkernel_sdk::CreateSandboxOptions;

// First create volumes via CLI: agentkernel volume create mydata

let opts = CreateSandboxOptions {
    image: Some("python:3.12-alpine".to_string()),
    volumes: vec!["mydata:/data".to_string(), "cache:/tmp/cache:ro".to_string()],
    ..Default::default()
};
client.create_sandbox("my-project", Some(opts)).await?;

// Data in /data persists across sandbox restarts
client.exec_in_sandbox("my-project", &["sh", "-c", "echo hello > /data/test.txt"], None).await?;
# Ok(())
# }
```

## File Operations

Read, write, and delete files. Batch write multiple files at once:

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
# let client = agentkernel_sdk::AgentKernel::builder().build()?;
use std::collections::HashMap;

// Single file
client.write_file("my-sandbox", "app/main.py", "print('hello')", None).await?;
let file = client.read_file("my-sandbox", "app/main.py").await?;
println!("{}", file.content);
client.delete_file("my-sandbox", "app/main.py").await?;

// Batch write
let mut files = HashMap::new();
files.insert("/app/index.js".to_string(), "console.log('hi')".to_string());
files.insert("/app/package.json".to_string(), r#"{"name":"app"}"#.to_string());
client.write_files("my-sandbox", files).await?;
# Ok(())
# }
```

## Detached Commands

Run long-lived processes in the background and retrieve their output later:

```rust,no_run
# async fn example() -> agentkernel_sdk::Result<()> {
# let client = agentkernel_sdk::AgentKernel::builder().build()?;
// Start a background process
let cmd = client.exec_detached("my-sandbox", &["python3", "train.py"], None).await?;
println!("Started: {} (pid {})", cmd.id, cmd.pid);

// Check status
let status = client.detached_status("my-sandbox", &cmd.id).await?;
println!("Status: {:?}", status.status);

// Get logs
let logs = client.detached_logs("my-sandbox", &cmd.id, None).await?;
if let Some(stdout) = &logs.stdout { println!("{}", stdout); }

// Get stderr only
let stderr = client.detached_logs("my-sandbox", &cmd.id, Some("stderr")).await?;

// List all detached commands
let all = client.detached_list("my-sandbox").await?;

// Kill a running command
client.detached_kill("my-sandbox", &cmd.id).await?;
# Ok(())
# }
```

## Configuration

```rust,no_run
use std::time::Duration;
use agentkernel_sdk::AgentKernel;

let client = AgentKernel::builder()
    .base_url("http://localhost:18888")
    .api_key("sk-...")
    .timeout(Duration::from_secs(60))
    .build()
    .unwrap();
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

## License

MIT
