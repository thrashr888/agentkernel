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

## Exec Options

Run commands with a working directory, environment variables, or as root:

```typescript
const result = await client.execInSandbox("my-sandbox", ["npm", "start"], {
  workdir: "/app",
  env: ["NODE_ENV=production"],
  sudo: true,
});
```

Works on sandbox sessions too:

```typescript
await using sb = await client.sandbox("dev");
await sb.run(["pip", "install", "-r", "requirements.txt"], {
  workdir: "/app",
  sudo: true,
});
```

## Git Source Cloning

Clone a git repo into the sandbox at creation time:

```typescript
const sb = await client.createSandbox("my-project", {
  image: "node:20-alpine",
  source_url: "https://github.com/user/repo.git",
  source_ref: "main",
});
```

## Persistent Volumes

Mount volumes that persist across sandbox restarts:

```typescript
// First create volumes via CLI: agentkernel volume create mydata

const sb = await client.createSandbox("my-project", {
  image: "node:20-alpine",
  volumes: ["mydata:/data", "cache:/tmp/cache:ro"],
});

// Data in /data persists across sandbox restarts
await sb.run(["sh", "-c", "echo hello > /data/test.txt"]);
```

## File Operations

Read, write, and delete files in a sandbox:

```typescript
// Write a file
await client.writeFile("my-sandbox", "app/main.py", "print('hello')");

// Read a file
const file = await client.readFile("my-sandbox", "app/main.py");
console.log(file.content);

// Delete a file
await client.deleteFile("my-sandbox", "app/main.py");

// Batch write multiple files at once
await client.writeFiles("my-sandbox", {
  "/app/index.js": "console.log('hi')",
  "/app/package.json": '{"name":"app"}',
});
```

## Detached Commands

Run long-lived processes in the background and retrieve their output later:

```typescript
// Start a background process
const cmd = await client.execDetached("my-sandbox", ["python3", "train.py"]);
console.log(`Started: ${cmd.id} (pid ${cmd.pid})`);

// Check status
const status = await client.detachedStatus("my-sandbox", cmd.id);
console.log(status.status); // "running" | "completed" | "failed"

// Get logs
const logs = await client.detachedLogs("my-sandbox", cmd.id);
console.log(logs.stdout);

// Get stderr only
const stderr = await client.detachedLogs("my-sandbox", cmd.id, "stderr");

// List all detached commands
const all = await client.detachedList("my-sandbox");

// Kill a running command
await client.detachedKill("my-sandbox", cmd.id);
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
  baseUrl: "http://localhost:18888", // default
  apiKey: "sk-...",                 // optional
  timeout: 30000,                   // default: 30s
});
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
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

Create a new sandbox. Options: `image`, `vcpus`, `memory_mb`, `profile`, `source_url`, `source_ref`, `volumes`.

### `client.getSandbox(name)`

Get sandbox info.

### `client.removeSandbox(name)`

Remove a sandbox.

### `client.execInSandbox(name, command, options?)`

Run a command in an existing sandbox. Options: `env`, `workdir`, `sudo`.

### `client.readFile(name, path)`

Read a file from a sandbox.

### `client.writeFile(name, path, content, options?)`

Write a file to a sandbox.

### `client.deleteFile(name, path)`

Delete a file from a sandbox.

### `client.writeFiles(name, files)`

Write multiple files to a sandbox in one request. `files` is a `Record<string, string>` of path to content.

### `client.execDetached(name, command, options?)`

Start a detached (background) command. Returns a `DetachedCommand`.

### `client.detachedStatus(name, cmdId)`

Get the status of a detached command.

### `client.detachedLogs(name, cmdId, stream?)`

Get stdout/stderr from a detached command. Pass `"stderr"` to get only stderr.

### `client.detachedKill(name, cmdId)`

Kill a detached command.

### `client.detachedList(name)`

List all detached commands in a sandbox.

### `client.sandbox(name, options?)`

Create a sandbox session with automatic cleanup. Returns a `SandboxSession` that implements `AsyncDisposable`.

## License

MIT
