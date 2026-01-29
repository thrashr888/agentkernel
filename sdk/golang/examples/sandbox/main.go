package main

import (
	"context"
	"fmt"
	"log"

	agentkernel "github.com/thrashr888/agentkernel/sdk/golang"
)

func main() {
	client := agentkernel.New(nil)
	ctx := context.Background()

	err := client.WithSandbox(ctx, "my-project", &agentkernel.CreateSandboxOptions{
		Image: "python:3.12-alpine",
	}, func(session *agentkernel.SandboxSession) error {
		output, err := session.Run(ctx, []string{"python3", "-c", "import sys; print(sys.version)"})
		if err != nil {
			return err
		}
		fmt.Print(output.Output)
		return nil
	})
	if err != nil {
		log.Fatal(err)
	}
	// sandbox auto-removed
}
