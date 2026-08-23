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
	Backend  string          `json:"backend,omitempty"`
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

// BackendCapabilities describes operations supported by a backend.
type BackendCapabilities struct {
	MountCWD            bool `json:"mount_cwd"`
	MountHome           bool `json:"mount_home"`
	Attach              bool `json:"attach"`
	HostVolumes         bool `json:"host_volumes"`
	SSH                 bool `json:"ssh"`
	ProxySecretBindings bool `json:"proxy_secret_bindings"`
	SecretFiles         bool `json:"secret_files"`
	Snapshots           bool `json:"snapshots"`
	Resume              bool `json:"resume"`
	Endpoints           bool `json:"endpoints"`
}

// BackendDescriptor describes one server backend.
type BackendDescriptor struct {
	Backend         string              `json:"backend"`
	Configured      bool                `json:"configured"`
	Usable          bool                `json:"usable"`
	ReadinessReason string              `json:"readiness_reason"`
	Capabilities    BackendCapabilities `json:"capabilities"`
}

// BackendDiscovery is returned by GET /backends.
type BackendDiscovery struct {
	DefaultBackend string              `json:"default_backend,omitempty"`
	Backends       []BackendDescriptor `json:"backends"`
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
	Backend     string          `json:"backend,omitempty"`
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

// Orchestration is an opaque durable orchestration payload.
type Orchestration map[string]interface{}

// CreateOrchestrationRequest is the request body for POST /orchestrations.
type CreateOrchestrationRequest map[string]interface{}

// OrchestrationDefinition is an opaque orchestration definition payload.
type OrchestrationDefinition map[string]interface{}

// DurableObject is an opaque durable object payload.
type DurableObject map[string]interface{}

// CreateObjectRequest is the request body for POST /objects.
type CreateObjectRequest map[string]interface{}

// Schedule is an opaque durable schedule payload.
type Schedule map[string]interface{}

// CreateScheduleRequest is the request body for POST /schedules.
type CreateScheduleRequest map[string]interface{}

// DurableStore is an opaque durable store payload.
type DurableStore map[string]interface{}

// CreateStoreRequest is the request body for POST /stores.
type CreateStoreRequest map[string]interface{}

// DurableStoreQueryResult is the response from POST /stores/{id}/query.
type DurableStoreQueryResult map[string]interface{}

// DurableStoreExecuteResult is the response from POST /stores/{id}/execute.
type DurableStoreExecuteResult map[string]interface{}

// DurableStoreCommandResult is the response from POST /stores/{id}/command.
type DurableStoreCommandResult map[string]interface{}

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
