
# Go SDK

Official Go client for agentkernel. Zero external dependencies — uses only the standard library.

- **Module**: `github.com/thrashr888/agentkernel/sdk/golang`
- **Source**: [`sdk/golang/`](https://github.com/thrashr888/agentkernel/tree/main/sdk/golang)
- **Requires**: Go 1.22+

## Install

```bash
go get github.com/thrashr888/agentkernel/sdk/golang
```

## Quick Start

```go
package main

import (
	"context"
	"fmt"
	"log"

	agentkernel "github.com/thrashr888/agentkernel/sdk/golang"
)

func main() {
	client := agentkernel.New(nil)

	output, err := client.Run(context.Background(), []string{"echo", "hello"}, nil)
	if err != nil {
		log.Fatal(err)
	}
	fmt.Print(output.Output) // "hello\n"
}
```

## Configuration

```go
client := agentkernel.New(&agentkernel.Options{
	BaseURL: "http://localhost:18888", // default
	APIKey:  "sk-...",               // optional
	Timeout: 60 * time.Second,       // default: 30s
})
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

For testing, inject a custom `http.Client`:

```go
client := agentkernel.New(&agentkernel.Options{
	BaseURL:    server.URL,
	HTTPClient: server.Client(),
})
```

## Running Commands

### Basic Execution

```go
output, err := client.Run(ctx, []string{"python3", "-c", "print(1 + 1)"}, nil)
fmt.Print(output.Output) // "2\n"
```

### With Options

```go
fast := false
opts := &agentkernel.RunOptions{
	Image:   "node:22-alpine",
	Profile: agentkernel.ProfileRestrictive,
	Fast:    &fast,
}
output, err := client.Run(ctx, []string{"npm", "test"}, opts)
```

### Streaming Output

Returns a channel of `StreamEvent`. The channel closes when the stream ends:

```go
ch, err := client.RunStream(ctx, []string{"python3", "script.py"}, nil)
if err != nil {
	log.Fatal(err)
}
for event := range ch {
	switch event.Type {
	case "output":
		fmt.Print(event.Data["content"])
	case "done":
		fmt.Println("Exit code:", event.Data["exit_code"])
	case "error":
		fmt.Println("Error:", event.Data["message"])
	}
}
```

## Sandbox Management

### Create and Execute

```go
// Create a sandbox
sandbox, err := client.CreateSandbox(ctx, "my-project", &agentkernel.CreateSandboxOptions{
	Image:    "python:3.12-alpine",
	VCPUs:    2,
	MemoryMB: 1024,
	Profile:  agentkernel.ProfileModerate,
})

// Execute commands
result, err := client.ExecInSandbox(ctx, "my-project", []string{"pip", "install", "numpy"})

// Get info
info, err := client.GetSandbox(ctx, "my-project")

// List all
sandboxes, err := client.ListSandboxes(ctx)

// Remove
err = client.RemoveSandbox(ctx, "my-project")
```

### Scoped Sandboxes (Recommended)

`WithSandbox` creates a sandbox, passes a `SandboxSession` to your callback, and removes the sandbox when done — even if the callback returns an error:

```go
err := client.WithSandbox(ctx, "test", &agentkernel.CreateSandboxOptions{
	Image: "python:3.12-alpine",
}, func(session *agentkernel.SandboxSession) error {
	session.Run(ctx, []string{"pip", "install", "numpy"})
	output, err := session.Run(ctx, []string{"python3", "-c", "import numpy; print(numpy.__version__)"})
	if err != nil {
		return err
	}
	fmt.Print(output.Output)
	return nil
})
// sandbox auto-removed
```

## File Operations

```go
// Read a file
file, _ := client.ReadFile(ctx, "my-sandbox", "tmp/hello.txt")
fmt.Println(file.Content)

// Write a file
client.WriteFile(ctx, "my-sandbox", "tmp/hello.txt", "hello world", "")

// Delete a file
client.DeleteFile(ctx, "my-sandbox", "tmp/hello.txt")
```

## Batch Execution

```go
results, _ := client.BatchRun(ctx, []agentkernel.BatchCommand{
	{Command: []string{"echo", "hello"}},
})
```

## Error Handling

Errors from the API are returned as `*agentkernel.Error` with the HTTP status code and server message:

```go
output, err := client.Run(ctx, []string{"bad-command"}, nil)
if err != nil {
	var apiErr *agentkernel.Error
	if errors.As(err, &apiErr) {
		fmt.Println(apiErr.StatusCode) // 400, 401, 404, 500, etc.
		fmt.Println(apiErr.Message)    // Error message from server
	}
}
```

Helper functions for common error types:

```go
if agentkernel.IsAuthError(err) {
	// 401 — invalid or missing API key
}
if agentkernel.IsValidationError(err) {
	// 400 — invalid request
}
if agentkernel.IsNotFoundError(err) {
	// 404 — sandbox not found
}
if agentkernel.IsServerError(err) {
	// 500+ — server error
}
```

## Context Support

All methods accept `context.Context` as the first parameter for cancellation and timeouts:

```go
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
defer cancel()

output, err := client.Run(ctx, []string{"echo", "hello"}, nil)
```

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `Health(ctx)` | `(string, error)` | Health check |
| `Run(ctx, command, opts)` | `(*RunOutput, error)` | Run command in temporary sandbox |
| `RunStream(ctx, command, opts)` | `(<-chan StreamEvent, error)` | Run with streaming output |
| `ListSandboxes(ctx)` | `([]SandboxInfo, error)` | List all sandboxes |
| `CreateSandbox(ctx, name, opts)` | `(*SandboxInfo, error)` | Create a sandbox |
| `GetSandbox(ctx, name)` | `(*SandboxInfo, error)` | Get sandbox info |
| `RemoveSandbox(ctx, name)` | `error` | Remove a sandbox |
| `ExecInSandbox(ctx, name, command)` | `(*RunOutput, error)` | Execute in existing sandbox |
| `ReadFile(ctx, name, path)` | `(*FileReadResponse, error)` | Read a file from a sandbox |
| `WriteFile(ctx, name, path, content, encoding)` | `(string, error)` | Write a file to a sandbox |
| `DeleteFile(ctx, name, path)` | `(string, error)` | Delete a file from a sandbox |
| `GetSandboxLogs(ctx, name)` | `([]LogEntry, error)` | Get sandbox audit logs |
| `BatchRun(ctx, commands)` | `(*BatchRunResponse, error)` | Run commands in parallel |
| `WithSandbox(ctx, name, opts, fn)` | `error` | Scoped session with auto-cleanup |
