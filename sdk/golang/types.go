package agentkernel

// SecurityProfile controls sandbox permissions.
type SecurityProfile string

const (
	ProfilePermissive  SecurityProfile = "permissive"
	ProfileModerate    SecurityProfile = "moderate"
	ProfileRestrictive SecurityProfile = "restrictive"
)

// RunOptions configures a run command.
type RunOptions struct {
	Image   string          `json:"image,omitempty"`
	Profile SecurityProfile `json:"profile,omitempty"`
	Fast    *bool           `json:"fast,omitempty"`
}

// CreateSandboxOptions configures sandbox creation.
type CreateSandboxOptions struct {
	Image string `json:"image,omitempty"`
}

// RunOutput is the result of a run or exec command.
type RunOutput struct {
	Output string `json:"output"`
}

// SandboxInfo describes a sandbox.
type SandboxInfo struct {
	Name    string `json:"name"`
	Status  string `json:"status"`
	Backend string `json:"backend,omitempty"`
}

// StreamEvent is a server-sent event from a streaming run.
type StreamEvent struct {
	Type string                 `json:"type"`
	Data map[string]interface{} `json:"data,omitempty"`
}

// apiResponse wraps all API responses.
type apiResponse[T any] struct {
	Success bool   `json:"success"`
	Data    T      `json:"data,omitempty"`
	Error   string `json:"error,omitempty"`
}

// runRequest is the POST /run body.
type runRequest struct {
	Command []string        `json:"command"`
	Image   string          `json:"image,omitempty"`
	Profile SecurityProfile `json:"profile,omitempty"`
	Fast    bool            `json:"fast"`
}

// createRequest is the POST /sandboxes body.
type createRequest struct {
	Name  string `json:"name"`
	Image string `json:"image,omitempty"`
}

// execRequest is the POST /sandboxes/{name}/exec body.
type execRequest struct {
	Command []string `json:"command"`
}
