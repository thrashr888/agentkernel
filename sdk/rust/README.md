# agentkernel-sdk

Rust SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run AI coding agents in secure, isolated microVMs.

## Install

```toml
[dependencies]
agentkernel-sdk = "0.1"
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
let client = agentkernel_sdk::AgentKernel::builder().build()?;

client.with_sandbox("test", Some("python:3.12-alpine"), |sb| async move {
    sb.run(&["pip", "install", "numpy"]).await?;
    let result = sb.run(&["python3", "-c", "import numpy; print(numpy.__version__)"]).await?;
    println!("{}", result.output);
    Ok(())
}).await?;
// sandbox auto-removed
# Ok(())
# }
```

## Configuration

```rust,no_run
use std::time::Duration;
use agentkernel_sdk::AgentKernel;

let client = AgentKernel::builder()
    .base_url("http://localhost:8880")
    .api_key("sk-...")
    .timeout(Duration::from_secs(60))
    .build()
    .unwrap();
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:8880
export AGENTKERNEL_API_KEY=sk-...
```

## License

MIT
