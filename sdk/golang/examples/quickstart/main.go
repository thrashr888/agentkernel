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
	fmt.Print(output.Output)
}
