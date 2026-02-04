# agentkernel Swift SDK

Swift client for the agentkernel API. Zero third-party dependencies.

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
print(output.output)
```

## Configuration

```swift
// Explicit options
let client = AgentKernel(AgentKernelOptions(
    baseURL: "http://localhost:18888",
    apiKey: "sk-...",
    timeout: 30
))

// Or use environment variables:
// AGENTKERNEL_BASE_URL, AGENTKERNEL_API_KEY
let client = AgentKernel()
```

## Usage

### Run a Command

```swift
let output = try await client.run(["python", "-c", "print('hi')"])
print(output.output)
```

With options:

```swift
let opts = RunOptions(image: "python:3.12-alpine", profile: .restrictive, fast: false)
let output = try await client.run(["python", "-c", "print('hi')"], options: opts)
```

### Stream Output (SSE)

```swift
let stream = try await client.runStream(["long-running-task"])
for try await event in stream {
    print("[\(event.eventType)] \(event.data)")
}
```

### Sandbox Lifecycle

```swift
// Create
let sb = try await client.createSandbox("my-sandbox")

// Execute
let output = try await client.execInSandbox("my-sandbox", command: ["ls", "-la"])

// Info
let info = try await client.getSandbox("my-sandbox")

// List all
let all = try await client.listSandboxes()

// Remove
try await client.removeSandbox("my-sandbox")
```

### Scoped Sandbox (Recommended)

`withSandbox` guarantees cleanup even if the closure throws:

```swift
let result = try await client.withSandbox("temp", options: CreateSandboxOptions(
    image: "node:20-alpine"
)) { session in
    let output = try await session.run(["node", "-e", "console.log('hi')"])
    return output.output
}
```

### Exec Options

Run commands with a working directory, environment variables, or as root:

```swift
let output = try await client.execInSandbox("my-sandbox", command: ["npm", "start"], options: ExecOptions(
    env: ["NODE_ENV=production"],
    workdir: "/app",
    sudo: true
))
```

Works on sandbox sessions too:

```swift
try await client.withSandbox("dev") { session in
    try await session.run(["pip", "install", "-r", "requirements.txt"], options: ExecOptions(
        workdir: "/app",
        sudo: true
    ))
}
```

### Git Source Cloning

Clone a git repo into the sandbox at creation time:

```swift
let sb = try await client.createSandbox("my-project", options: CreateSandboxOptions(
    image: "node:20-alpine",
    sourceURL: "https://github.com/user/repo.git",
    sourceRef: "main"
))
```

### File Operations

Read, write, and delete files in a sandbox:

```swift
// Write a file
try await client.writeFile("my-sandbox", path: "app/main.py", content: "print('hello')")

// Read a file
let file = try await client.readFile("my-sandbox", path: "app/main.py")
print(file.content)

// Delete a file
try await client.deleteFile("my-sandbox", path: "app/main.py")

// Batch write multiple files at once
try await client.writeFiles("my-sandbox", files: [
    "/app/index.js": "console.log('hi')",
    "/app/package.json": "{\"name\":\"app\"}"
])
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

## Requirements

- Swift 5.9+
- macOS 13+ / iOS 16+ / Linux

## License

MIT
