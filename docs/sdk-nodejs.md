
# Node.js SDK

Official Node.js client for agentkernel. Zero HTTP dependencies — uses native `fetch`.

- **Package**: [`agentkernel`](https://www.npmjs.com/package/agentkernel)
- **Source**: [`sdk/nodejs/`](https://github.com/thrashr888/agentkernel/tree/main/sdk/nodejs)
- **Requires**: Node.js 20+

## Install

```bash
npm install agentkernel
```

## Quick Start

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Run a command in a temporary sandbox
const result = await client.run(["echo", "hello"]);
console.log(result.output); // "hello\n"
```

## Configuration

```typescript
const client = new AgentKernel({
  baseUrl: "http://localhost:8880", // default
  apiKey: "sk-...",                 // optional
  timeout: 30000,                   // default: 30s
});
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:8880
export AGENTKERNEL_API_KEY=sk-...
```

## Running Commands

### Basic Execution

```typescript
const result = await client.run(["python3", "-c", "print(1 + 1)"]);
console.log(result.output); // "2\n"
```

### With Options

```typescript
const result = await client.run(["npm", "test"], {
  image: "node:22-alpine",
  profile: "restrictive",
  fast: false,
});
```

### Streaming Output

Returns an `AsyncGenerator<StreamEvent>` for real-time output:

```typescript
for await (const event of client.runStream(["python3", "script.py"])) {
  switch (event.type) {
    case "output":
      process.stdout.write(String(event.data.data));
      break;
    case "done":
      console.log("Exit code:", event.data.exit_code);
      break;
    case "error":
      console.error("Error:", event.data.message);
      break;
  }
}
```

## Sandbox Management

### Create and Execute

```typescript
// Create a sandbox
const sandbox = await client.createSandbox("my-project", {
  image: "python:3.12-alpine",
});

// Execute commands
const result = await client.execInSandbox("my-project", ["pip", "install", "numpy"]);

// Get info
const info = await client.getSandbox("my-project");

// List all
const sandboxes = await client.listSandboxes();

// Remove
await client.removeSandbox("my-project");
```

### Sandbox Sessions (Recommended)

The `sandbox()` method returns a `SandboxSession` that implements `AsyncDisposable`. The sandbox is automatically removed when the scope exits:

```typescript
await using sb = await client.sandbox("my-project", {
  image: "python:3.12-alpine",
});

await sb.run(["pip", "install", "numpy"]);
const result = await sb.run(["python3", "-c", "import numpy; print(numpy.__version__)"]);
console.log(result.output);
// sandbox auto-removed when scope exits
```

For environments without `await using` support:

```typescript
const sb = await client.sandbox("my-project");
try {
  const result = await sb.run(["echo", "hello"]);
  console.log(result.output);
} finally {
  await sb[Symbol.asyncDispose]();
}
```

## Error Handling

```typescript
import { AgentKernelError } from "agentkernel";

try {
  await client.run(["bad-command"]);
} catch (error) {
  if (error instanceof AgentKernelError) {
    console.log(error.status);  // HTTP status code
    console.log(error.message); // Error message from server
  }
}
```

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `health()` | `Promise<string>` | Health check |
| `run(command, options?)` | `Promise<RunOutput>` | Run command in temporary sandbox |
| `runStream(command, options?)` | `AsyncGenerator<StreamEvent>` | Run with streaming output |
| `listSandboxes()` | `Promise<SandboxInfo[]>` | List all sandboxes |
| `createSandbox(name, options?)` | `Promise<SandboxInfo>` | Create a sandbox |
| `getSandbox(name)` | `Promise<SandboxInfo>` | Get sandbox info |
| `removeSandbox(name)` | `Promise<void>` | Remove a sandbox |
| `execInSandbox(name, command)` | `Promise<RunOutput>` | Execute in existing sandbox |
| `sandbox(name, options?)` | `Promise<SandboxSession>` | Scoped session with auto-cleanup |
