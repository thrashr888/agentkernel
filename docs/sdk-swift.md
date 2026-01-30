
# Swift SDK

Official Swift client for agentkernel. Actor-based concurrency, zero third-party dependencies.

- **Package**: `AgentKernel` via Swift Package Manager
- **Source**: [`sdk/swift/`](https://github.com/thrashr888/agentkernel/tree/main/sdk/swift)
- **Requires**: Swift 5.9+, macOS 13+ / iOS 16+ / Linux

## Install

Add to your `Package.swift`:

```swift
dependencies: [
    .package(url: "https://github.com/thrashr888/agentkernel.git", from: "0.1.0")
]
```

Then add the dependency to your target:

```swift
.target(name: "MyApp", dependencies: [
    .product(name: "AgentKernel", package: "agentkernel")
])
```

## Quick Start

```swift
import AgentKernel

let client = AgentKernel()

let output = try await client.run(["echo", "hello"])
print(output.output) // "hello\n"
```

## Configuration

### Explicit Options

```swift
let client = AgentKernel(AgentKernelOptions(
    baseURL: "http://localhost:18888",
    apiKey: "sk-...",
    timeout: 30
))
```

### Environment Variables

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

```swift
// Picks up env vars automatically
let client = AgentKernel()
```

## Running Commands

### Basic Execution

```swift
let output = try await client.run(["python3", "-c", "print(1 + 1)"])
print(output.output) // "2\n"
```

### With Options

```swift
let opts = RunOptions(
    image: "node:22-alpine",
    profile: .restrictive,
    fast: false
)
let output = try await client.run(["npm", "test"], options: opts)
```

### Streaming Output

Returns an `AsyncThrowingStream<StreamEvent, Error>`:

```swift
let stream = try await client.runStream(["python3", "script.py"])
for try await event in stream {
    switch event.eventType {
    case "output":
        print(event.data["data"] ?? "", terminator: "")
    case "done":
        print("Exit code: \(event.data["exit_code"] ?? "")")
    case "error":
        print("Error: \(event.data["message"] ?? "")")
    default:
        break
    }
}
```

## Sandbox Management

### Create and Execute

```swift
// Create a sandbox
let sandbox = try await client.createSandbox("my-project",
    options: CreateSandboxOptions(image: "python:3.12-alpine"))

// Execute commands
let result = try await client.execInSandbox("my-project",
    command: ["pip", "install", "numpy"])

// Get info
let info = try await client.getSandbox("my-project")

// List all
let sandboxes = try await client.listSandboxes()

// Remove
try await client.removeSandbox("my-project")
```

### Scoped Sandboxes (Recommended)

`withSandbox` creates a sandbox, passes a session to your closure, and removes it when done — even if the closure throws:

```swift
let result = try await client.withSandbox("test", image: "python:3.12-alpine") { session in
    try await session.run(["pip", "install", "numpy"])
    let output = try await session.run(["python3", "-c", "import numpy; print(numpy.__version__)"])
    return output.output
}
print(result)
// sandbox auto-removed
```

## Error Handling

```swift
do {
    let output = try await client.run(["bad-command"])
} catch let error as AgentKernelError {
    switch error {
    case .auth(let msg):       print("Auth: \(msg)")
    case .validation(let msg): print("Validation: \(msg)")
    case .notFound(let msg):   print("Not found: \(msg)")
    case .server(let msg):     print("Server: \(msg)")
    case .network(let err):    print("Network: \(err)")
    case .stream(let msg):     print("Stream: \(msg)")
    case .json(let err):       print("JSON: \(err)")
    }
}
```

| Error Case | HTTP Status | Description |
|------------|-------------|-------------|
| `.auth` | 401 | Invalid or missing API key |
| `.validation` | 400 | Invalid request |
| `.notFound` | 404 | Sandbox not found |
| `.server` | 500+ | Server error |
| `.network` | — | Connection failure |
| `.stream` | — | SSE parsing error |
| `.json` | — | Response decode error |

## Types

### `RunOutput`

```swift
public struct RunOutput: Codable, Sendable {
    public let output: String
}
```

### `SandboxInfo`

```swift
public struct SandboxInfo: Codable, Sendable {
    public let name: String
    public let status: String
    public let backend: String?
}
```

### `SecurityProfile`

```swift
public enum SecurityProfile: String, Codable, Sendable {
    case permissive
    case moderate
    case restrictive
}
```

## Thread Safety

`AgentKernel` is declared as a Swift `actor`, making all methods safe to call from any concurrency context. No manual synchronization needed.

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `health()` | `String` | Health check |
| `run(command, options?)` | `RunOutput` | Run command in temporary sandbox |
| `runStream(command, options?)` | `AsyncThrowingStream<StreamEvent, Error>` | Run with streaming output |
| `listSandboxes()` | `[SandboxInfo]` | List all sandboxes |
| `createSandbox(name, options?)` | `SandboxInfo` | Create a sandbox |
| `getSandbox(name)` | `SandboxInfo` | Get sandbox info |
| `removeSandbox(name)` | `Void` | Remove a sandbox |
| `execInSandbox(name, command:)` | `RunOutput` | Execute in existing sandbox |
| `withSandbox(name, image?, body)` | `T` | Scoped session with auto-cleanup |

All methods are `async throws`.
