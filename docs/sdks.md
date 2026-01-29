
# SDKs

agentkernel provides official SDK clients in five languages. Each SDK wraps the [HTTP API](api-http.html) with idiomatic language bindings.

## Quick Comparison

| SDK | Package | Install | Async | Streaming | Sandbox Sessions |
|-----|---------|---------|-------|-----------|------------------|
| [Node.js](sdk-nodejs.html) | `agentkernel` | `npm install agentkernel` | Native (Promise) | `AsyncGenerator` | `await using` (auto-cleanup) |
| [Python](sdk-python.html) | `agentkernel` | `pip install agentkernel` | Sync + Async | `Iterator` / `AsyncIterator` | Context manager |
| [Go](sdk-golang.html) | `agentkernel` | `go get github.com/thrashr888/agentkernel/sdk/golang` | `context.Context` | `<-chan StreamEvent` | Callback with `WithSandbox` |
| [Rust](sdk-rust.html) | `agentkernel-sdk` | `cargo add agentkernel-sdk` | `async`/`await` | `Stream` | Closure with `with_sandbox` |
| [Swift](sdk-swift.html) | `AgentKernel` | Swift Package Manager | `async`/`await` | `AsyncThrowingStream` | Closure with `withSandbox` |

All SDKs share the same API surface:

- **`health()`** — Health check
- **`run(command, options?)`** — Run a command in a temporary sandbox
- **`runStream(command, options?)`** — Run with streaming output (SSE)
- **`listSandboxes()`** — List all sandboxes
- **`createSandbox(name, options?)`** — Create a sandbox
- **`getSandbox(name)`** — Get sandbox info
- **`removeSandbox(name)`** — Remove a sandbox
- **`execInSandbox(name, command)`** — Execute in an existing sandbox
- **`sandbox(name)` / `withSandbox(name)`** — Scoped session with automatic cleanup

## Configuration

Every SDK resolves configuration the same way:

1. **Explicit options** passed to the constructor
2. **Environment variables** (`AGENTKERNEL_BASE_URL`, `AGENTKERNEL_API_KEY`)
3. **Defaults** (`http://localhost:8080`, no API key, 30s timeout)

## Quick Start by Language

### Node.js

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();
const result = await client.run(["echo", "hello"]);
console.log(result.output);
```

### Python

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    result = client.run(["echo", "hello"])
    print(result.output)
```

### Rust

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

### Go

```go
import agentkernel "github.com/thrashr888/agentkernel/sdk/golang"

client := agentkernel.New(nil)
output, _ := client.Run(context.Background(), []string{"echo", "hello"}, nil)
fmt.Print(output.Output)
```

### Swift

```swift
import AgentKernel

let client = AgentKernel()
let output = try await client.run(["echo", "hello"])
print(output.output)
```

## Source Code

All SDKs live in the [`sdk/`](https://github.com/thrashr888/agentkernel/tree/main/sdk) directory:

```
sdk/
  nodejs/     → npm (agentkernel)
  python/     → PyPI (agentkernel)
  golang/     → Go module (github.com/thrashr888/agentkernel/sdk/golang)
  rust/       → crates.io (agentkernel-sdk)
  swift/      → Swift Package Manager (AgentKernel)
```
