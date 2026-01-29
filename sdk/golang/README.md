# agentkernel

Go SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run AI coding agents in secure, isolated microVMs.

## Install

```bash
go get github.com/thrashr888/agentkernel/sdk/golang
```

Requires Go 1.22+. Zero external dependencies.

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

## Sandbox Sessions

```go
err := client.WithSandbox(ctx, "test", &agentkernel.CreateSandboxOptions{
	Image: "python:3.12-alpine",
}, func(session *agentkernel.SandboxSession) error {
	session.Run(ctx, []string{"pip", "install", "numpy"})
	output, _ := session.Run(ctx, []string{"python3", "-c", "import numpy; print(numpy.__version__)"})
	fmt.Print(output.Output)
	return nil
})
// sandbox auto-removed
```

## Streaming

```go
ch, err := client.RunStream(ctx, []string{"python3", "script.py"}, nil)
if err != nil {
	log.Fatal(err)
}
for event := range ch {
	if event.Type == "output" {
		fmt.Print(event.Data["content"])
	}
}
```

## Configuration

```go
client := agentkernel.New(&agentkernel.Options{
	BaseURL: "http://localhost:8080", // default
	APIKey:  "sk-...",               // optional
	Timeout: 60 * time.Second,       // default: 30s
})
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:8080
export AGENTKERNEL_API_KEY=sk-...
```

## License

MIT
