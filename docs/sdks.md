
# SDKs

agentkernel provides official SDK clients in five languages. Each SDK wraps the [HTTP API](../api-http) with idiomatic language bindings.

## Quick Comparison

| SDK | Package | Registry | Install |
|-----|---------|----------|---------|
| [Node.js](../sdk-nodejs) | `agentkernel` | [npm](https://www.npmjs.com/package/agentkernel) | `npm install agentkernel` |
| [Python](../sdk-python) | `agentkernel-sdk` | [PyPI](https://pypi.org/project/agentkernel-sdk/) | `pip install agentkernel-sdk` |
| [Go](../sdk-golang) | `agentkernel` | [pkg.go.dev](https://pkg.go.dev/github.com/thrashr888/agentkernel/sdk/golang) | `go get github.com/thrashr888/agentkernel/sdk/golang` |
| [Rust](../sdk-rust) | `agentkernel-sdk` | [crates.io](https://crates.io/crates/agentkernel-sdk) | `cargo add agentkernel-sdk` |
| [Swift](../sdk-swift) | `AgentKernel` | [GitHub](https://github.com/thrashr888/agentkernel/tree/main/sdk/swift) | Swift Package Manager |

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
3. **Defaults** (`http://localhost:18888`, no API key, 30s timeout)

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
