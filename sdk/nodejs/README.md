# agentkernel

Node.js SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run AI coding agents in secure, isolated microVMs.

## Install

```bash
npm install agentkernel
```

Requires Node.js 20+. Zero HTTP dependencies (uses native `fetch`).

## Quick Start

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Run a command in a temporary sandbox
const result = await client.run(["echo", "hello"]);
console.log(result.output); // "hello\n"
```

## Sandbox Sessions

Create a persistent sandbox with automatic cleanup:

```typescript
await using sb = await client.sandbox("my-project", {
  image: "python:3.12-alpine",
});

await sb.run(["pip", "install", "numpy"]);
const result = await sb.run(["python3", "-c", "import numpy; print(numpy.__version__)"]);
console.log(result.output);
// sandbox auto-removed when scope exits
```

## Streaming

```typescript
for await (const event of client.runStream(["python3", "script.py"])) {
  if (event.type === "output") process.stdout.write(String(event.data.data));
}
```

## Configuration

```typescript
const client = new AgentKernel({
  baseUrl: "http://localhost:8080", // default
  apiKey: "sk-...",                 // optional
  timeout: 30000,                   // default: 30s
});
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:8080
export AGENTKERNEL_API_KEY=sk-...
```

## API

### `client.health()`

Health check. Returns `"ok"`.

### `client.run(command, options?)`

Run a command in a temporary sandbox.

```typescript
await client.run(["echo", "hello"]);
await client.run(["python3", "-c", "print(1)"], {
  image: "python:3.12-alpine",
  profile: "restrictive",
  fast: false,
});
```

### `client.runStream(command, options?)`

Run a command with SSE streaming output. Returns an `AsyncGenerator<StreamEvent>`.

### `client.listSandboxes()`

List all sandboxes.

### `client.createSandbox(name, options?)`

Create a new sandbox.

### `client.getSandbox(name)`

Get sandbox info.

### `client.removeSandbox(name)`

Remove a sandbox.

### `client.execInSandbox(name, command)`

Run a command in an existing sandbox.

### `client.sandbox(name, options?)`

Create a sandbox session with automatic cleanup. Returns a `SandboxSession` that implements `AsyncDisposable`.

## License

MIT
