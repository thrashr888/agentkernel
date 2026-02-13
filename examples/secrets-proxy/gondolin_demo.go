// Gondolin-style secrets demo for agentkernel.
//
// Demonstrates the same pattern as github.com/earendil-works/gondolin:
// secrets are injected as HTTP headers by the host proxy and never
// enter the sandbox VM. The sandbox only sees placeholder env vars.
//
// Usage:
//
//	export GITHUB_TOKEN="ghp_..."
//	go run examples/secrets-proxy/gondolin_demo.go
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	agentkernel "github.com/thrashr888/agentkernel/sdk/golang"
)

const sandboxName = "gondolin-demo"

func main() {
	token := os.Getenv("GITHUB_TOKEN")
	if token == "" {
		fmt.Fprintln(os.Stderr, "Set GITHUB_TOKEN env var first:")
		fmt.Fprintln(os.Stderr, `  export GITHUB_TOKEN="ghp_..."`)
		os.Exit(1)
	}

	ctx := context.Background()
	client := agentkernel.New(nil)

	// Create sandbox with secret binding (Gondolin pattern).
	// The proxy intercepts HTTPS requests to api.github.com and injects
	// an Authorization header with the real token. The VM never sees it.
	fmt.Println("Creating sandbox with secret binding...")
	_, err := client.CreateSandbox(ctx, sandboxName, &agentkernel.CreateSandboxOptions{
		Image:   "python:3.12-slim",
		Profile: agentkernel.ProfileModerate,
		Secrets: []string{fmt.Sprintf("GITHUB_TOKEN=%s:api.github.com", token)},
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to create sandbox: %v\n", err)
		os.Exit(1)
	}
	defer client.RemoveSandbox(ctx, sandboxName) //nolint:errcheck

	// 1. Verify the sandbox doesn't have the real token
	fmt.Println("\n--- Env check (token should be a placeholder) ---")
	envCheck, err := client.ExecInSandbox(ctx, sandboxName, []string{
		"sh", "-c", "echo GITHUB_TOKEN=$GITHUB_TOKEN",
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "Env check failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Print(envCheck.Output)

	// 2. Make an authenticated API call through the proxy (HTTPS with MITM).
	//    The proxy transparently injects Authorization: Bearer <real-token>.
	fmt.Println("\n--- Calling GitHub API (secret injected by proxy) ---")
	result, err := client.ExecInSandbox(ctx, sandboxName, []string{
		"python3", "-c",
		`import urllib.request, json
req = urllib.request.Request("https://api.github.com/user",
    headers={"Accept": "application/vnd.github+json"})
resp = urllib.request.urlopen(req, timeout=15)
print(resp.read().decode())`,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "GitHub API call failed: %v\n", err)
		os.Exit(1)
	}
	var user map[string]interface{}
	if err := json.Unmarshal([]byte(result.Output), &user); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to parse response: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Authenticated as: %s (%s)\n", user["login"], user["name"])
	fmt.Printf("Public repos: %.0f\n", user["public_repos"])

	// 3. Try an unauthorized host — should be blocked by the proxy
	fmt.Println("\n--- Attempting unauthorized host (should fail) ---")
	_, err = client.ExecInSandbox(ctx, sandboxName, []string{
		"python3", "-c",
		"import urllib.request; urllib.request.urlopen('https://evil.com', timeout=5)",
	})
	if err != nil {
		fmt.Println("Blocked as expected")
	} else {
		fmt.Println("ERROR: request to unauthorized host should have failed")
	}

	fmt.Println("\nDone. Secret never entered the VM.")
	fmt.Println("Sandbox removed.")
}
