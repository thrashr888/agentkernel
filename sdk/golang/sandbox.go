package agentkernel

import "context"

// SandboxSession provides a handle to run commands in an existing sandbox.
// Used within the Client.WithSandbox callback.
type SandboxSession struct {
	name   string
	client *Client
}

// Run executes a command in the sandbox.
func (s *SandboxSession) Run(ctx context.Context, command []string) (*RunOutput, error) {
	return s.client.ExecInSandbox(ctx, s.name, command)
}

// Info returns the sandbox's current info.
func (s *SandboxSession) Info(ctx context.Context) (*SandboxInfo, error) {
	return s.client.GetSandbox(ctx, s.name)
}

// Name returns the sandbox name.
func (s *SandboxSession) Name() string {
	return s.name
}
