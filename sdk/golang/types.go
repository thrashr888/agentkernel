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
	// Volumes are mount specs (slug:/path or slug:/path:ro). Create volumes via CLI first.
	Volumes []string `json:"volumes,omitempty"`
	// Secrets are proxy-based secret bindings (Gondolin pattern).
	// Secrets are injected as HTTP headers by the host proxy — they never enter the VM.
	// Formats: "KEY=value:host", "KEY:host", "KEY:host:header".
	Secrets []string `json:"secrets,omitempty"`
	// SecretFiles are secret keys to inject as files at /run/agentkernel/secrets/KEY.
	// Values are resolved from the secret vault.
	SecretFiles []string `json:"secret_files,omitempty"`
}

// RunOutput is the result of a run or exec command.
type RunOutput struct {
	Output string `json:"output"`
}

// SandboxInfo describes a sandbox.
type SandboxInfo struct {
	Name      string `json:"name"`
	UUID      string `json:"uuid,omitempty"`
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
	Name        string          `json:"name"`
	Image       string          `json:"image,omitempty"`
	VCPUs       int             `json:"vcpus,omitempty"`
	MemoryMB    int             `json:"memory_mb,omitempty"`
	Profile     SecurityProfile `json:"profile,omitempty"`
	Volumes     []string        `json:"volumes,omitempty"`
	Secrets     []string        `json:"secrets,omitempty"`
	SecretFiles []string        `json:"secret_files,omitempty"`
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

// ExtendTtlResponse is the response from extending a sandbox's TTL.
type ExtendTtlResponse struct {
	ExpiresAt string `json:"expires_at,omitempty"`
}

// extendTtlRequest is the POST /sandboxes/{name}/extend body.
type extendTtlRequest struct {
	By string `json:"by"`
}

// SnapshotMeta describes a snapshot.
type SnapshotMeta struct {
	Name      string `json:"name"`
	Sandbox   string `json:"sandbox"`
	ImageTag  string `json:"image_tag"`
	Backend   string `json:"backend"`
	BaseImage string `json:"base_image,omitempty"`
	VCPUs     int    `json:"vcpus,omitempty"`
	MemoryMB  int    `json:"memory_mb,omitempty"`
	CreatedAt string `json:"created_at"`
}

// TakeSnapshotOptions configures snapshot creation.
type TakeSnapshotOptions struct {
	Sandbox string `json:"sandbox"`
	Name    string `json:"name,omitempty"`
}

// PageLink is a hyperlink extracted from a web page.
type PageLink struct {
	Text string `json:"text"`
	Href string `json:"href"`
}

// PageResult contains the data returned by BrowserSession.Goto.
type PageResult struct {
	Title string     `json:"title"`
	URL   string     `json:"url"`
	Text  string     `json:"text"`
	Links []PageLink `json:"links"`
}

// AriaSnapshot is an ARIA accessibility tree snapshot of a web page (v2).
type AriaSnapshot struct {
	// ARIA tree as YAML.
	Snapshot string `json:"snapshot"`
	// Current page URL.
	URL string `json:"url"`
	// Page title.
	Title string `json:"title"`
	// Available ref IDs (e.g. ["e1", "e2"]).
	Refs []string `json:"refs"`
}

// BrowserEvent is a browser interaction event for debugging and context recovery.
type BrowserEvent struct {
	// Monotonic sequence number.
	Seq int `json:"seq"`
	// Event type (e.g. "page.navigated", "page.clicked").
	Type string `json:"type"`
	// Page name.
	Page string `json:"page"`
	// ISO 8601 timestamp.
	Ts string `json:"ts"`
}

// BrowserOption configures a browser session.
type BrowserOption func(*browserConfig)

type browserConfig struct {
	memoryMB int
}

// WithMemoryMB sets the sandbox memory in megabytes (default 2048).
func WithMemoryMB(mb int) BrowserOption {
	return func(cfg *browserConfig) {
		cfg.memoryMB = mb
	}
}
