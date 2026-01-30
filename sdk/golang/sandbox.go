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

// ReadFile reads a file from the sandbox.
func (s *SandboxSession) ReadFile(ctx context.Context, path string) (*FileReadResponse, error) {
	return s.client.ReadFile(ctx, s.name, path)
}

// WriteFile writes a file to the sandbox.
func (s *SandboxSession) WriteFile(ctx context.Context, path, content string, encoding string) error {
	return s.client.WriteFile(ctx, s.name, path, content, encoding)
}

// DeleteFile deletes a file from the sandbox.
func (s *SandboxSession) DeleteFile(ctx context.Context, path string) error {
	return s.client.DeleteFile(ctx, s.name, path)
}

// Logs returns audit log entries for the sandbox.
func (s *SandboxSession) Logs(ctx context.Context) ([]map[string]interface{}, error) {
	return s.client.GetSandboxLogs(ctx, s.name)
}
