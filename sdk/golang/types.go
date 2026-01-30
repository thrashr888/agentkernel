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
	Image    string          `json:"image,omitempty"`
	VCPUs    int             `json:"vcpus,omitempty"`
	MemoryMB int             `json:"memory_mb,omitempty"`
	Profile  SecurityProfile `json:"profile,omitempty"`
}

// RunOutput is the result of a run or exec command.
type RunOutput struct {
	Output string `json:"output"`
}

// SandboxInfo describes a sandbox.
type SandboxInfo struct {
	Name      string `json:"name"`
	Status    string `json:"status"`
	Backend   string `json:"backend,omitempty"`
	Image     string `json:"image,omitempty"`
	VCPUs     int    `json:"vcpus,omitempty"`
	MemoryMB  int    `json:"memory_mb,omitempty"`
	CreatedAt string `json:"created_at,omitempty"`
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
	Name     string          `json:"name"`
	Image    string          `json:"image,omitempty"`
	VCPUs    int             `json:"vcpus,omitempty"`
	MemoryMB int             `json:"memory_mb,omitempty"`
	Profile  SecurityProfile `json:"profile,omitempty"`
}

// execRequest is the POST /sandboxes/{name}/exec body.
type execRequest struct {
	Command []string `json:"command"`
}

// FileReadResponse contains the content of a file read from a sandbox.
type FileReadResponse struct {
	Content  string `json:"content"`
	Encoding string `json:"encoding"`
	Size     int    `json:"size"`
}

// BatchCommand is a single command in a batch execution request.
type BatchCommand struct {
	Command []string `json:"command"`
}

// BatchResult is the result of a single command in a batch.
type BatchResult struct {
	Output *string `json:"output"`
	Error  *string `json:"error"`
}

// BatchRunResponse is the response from batch execution.
type BatchRunResponse struct {
	Results []BatchResult `json:"results"`
}

// fileWriteRequest is the PUT /sandboxes/{name}/files/{path} body.
type fileWriteRequest struct {
	Content  string `json:"content"`
	Encoding string `json:"encoding,omitempty"`
}

// batchRunRequest is the POST /batch/run body.
type batchRunRequest struct {
	Commands []BatchCommand `json:"commands"`
}
