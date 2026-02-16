// Package agentkernel provides a Go client for the agentkernel HTTP API.
//
// Zero external dependencies — uses only the standard library.
package agentkernel

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"
)

const (
	sdkVersion     = "0.3.0"
	defaultBaseURL = "http://localhost:18888"
	defaultTimeout = 30 * time.Second
)

// Options configures the agentkernel client.
type Options struct {
	// BaseURL is the agentkernel server URL. Default: http://localhost:18888
	BaseURL string

	// APIKey is the optional API key for authentication.
	APIKey string

	// Timeout is the HTTP request timeout. Default: 30s.
	Timeout time.Duration

	// HTTPClient overrides the default http.Client. Useful for testing.
	HTTPClient *http.Client
}

// Client is the agentkernel API client.
type Client struct {
	baseURL    string
	apiKey     string
	httpClient *http.Client
}

// New creates a new agentkernel client.
//
// Configuration is resolved in order: explicit options > environment variables > defaults.
//
//	client := agentkernel.New(nil)                     // defaults + env vars
//	client := agentkernel.New(&agentkernel.Options{    // explicit
//	    BaseURL: "http://localhost:9090",
//	    APIKey:  "sk-...",
//	})
func New(opts *Options) *Client {
	baseURL := defaultBaseURL
	apiKey := ""
	timeout := defaultTimeout
	var httpClient *http.Client

	// Env vars
	if v := os.Getenv("AGENTKERNEL_BASE_URL"); v != "" {
		baseURL = v
	}
	if v := os.Getenv("AGENTKERNEL_API_KEY"); v != "" {
		apiKey = v
	}

	// Explicit options override
	if opts != nil {
		if opts.BaseURL != "" {
			baseURL = opts.BaseURL
		}
		if opts.APIKey != "" {
			apiKey = opts.APIKey
		}
		if opts.Timeout > 0 {
			timeout = opts.Timeout
		}
		httpClient = opts.HTTPClient
	}

	if httpClient == nil {
		httpClient = &http.Client{Timeout: timeout}
	}

	return &Client{
		baseURL:    strings.TrimRight(baseURL, "/"),
		apiKey:     apiKey,
		httpClient: httpClient,
	}
}

// Health returns "ok" if the server is healthy.
func (c *Client) Health(ctx context.Context) (string, error) {
	var result string
	err := c.request(ctx, http.MethodGet, "/health", nil, &result)
	return result, err
}

// Run executes a command in a temporary sandbox.
func (c *Client) Run(ctx context.Context, command []string, opts *RunOptions) (*RunOutput, error) {
	fast := true
	if opts != nil && opts.Fast != nil {
		fast = *opts.Fast
	}
	body := runRequest{
		Command: command,
		Fast:    fast,
	}
	if opts != nil {
		body.Image = opts.Image
		body.Profile = opts.Profile
	}
	var result RunOutput
	err := c.request(ctx, http.MethodPost, "/run", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// RunStream executes a command with SSE streaming output.
// Returns a channel of StreamEvent. The channel is closed when the stream ends.
func (c *Client) RunStream(ctx context.Context, command []string, opts *RunOptions) (<-chan StreamEvent, error) {
	fast := true
	if opts != nil && opts.Fast != nil {
		fast = *opts.Fast
	}
	body := runRequest{
		Command: command,
		Fast:    fast,
	}
	if opts != nil {
		body.Image = opts.Image
		body.Profile = opts.Profile
	}

	jsonBody, err := json.Marshal(body)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost,
		c.baseURL+"/run/stream", bytes.NewReader(jsonBody))
	if err != nil {
		return nil, err
	}
	c.applyHeaders(req)
	req.Header.Set("Accept", "text/event-stream")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 400 {
		defer resp.Body.Close()
		return nil, c.handleErrorResponse(resp)
	}

	// ParseSSE closes the channel when the body is exhausted.
	// Wrap the body so it gets closed when parsing is done.
	ch := make(chan StreamEvent)
	go func() {
		defer close(ch)
		defer resp.Body.Close()
		for event := range ParseSSE(resp.Body) {
			ch <- event
		}
	}()
	return ch, nil
}

// ListSandboxes returns all sandboxes.
func (c *Client) ListSandboxes(ctx context.Context) ([]SandboxInfo, error) {
	var result []SandboxInfo
	err := c.request(ctx, http.MethodGet, "/sandboxes", nil, &result)
	return result, err
}

// CreateSandbox creates a new sandbox.
func (c *Client) CreateSandbox(ctx context.Context, name string, opts *CreateSandboxOptions) (*SandboxInfo, error) {
	body := createRequest{Name: name}
	if opts != nil {
		body.Image = opts.Image
		body.VCPUs = opts.VCPUs
		body.MemoryMB = opts.MemoryMB
		body.Profile = opts.Profile
		body.Volumes = opts.Volumes
		body.Secrets = opts.Secrets
		body.SecretFiles = opts.SecretFiles
	}
	var result SandboxInfo
	err := c.request(ctx, http.MethodPost, "/sandboxes", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetSandbox returns info about a sandbox.
func (c *Client) GetSandbox(ctx context.Context, name string) (*SandboxInfo, error) {
	var result SandboxInfo
	err := c.request(ctx, http.MethodGet, "/sandboxes/"+name, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetSandboxByUUID returns info about a sandbox by UUID.
func (c *Client) GetSandboxByUUID(ctx context.Context, uuid string) (*SandboxInfo, error) {
	var result SandboxInfo
	err := c.request(ctx, http.MethodGet, "/sandboxes/by-uuid/"+uuid, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// RemoveSandbox removes a sandbox.
func (c *Client) RemoveSandbox(ctx context.Context, name string) error {
	var result string
	return c.request(ctx, http.MethodDelete, "/sandboxes/"+name, nil, &result)
}

// ExecInSandbox executes a command in an existing sandbox.
func (c *Client) ExecInSandbox(ctx context.Context, name string, command []string) (*RunOutput, error) {
	body := execRequest{Command: command}
	var result RunOutput
	err := c.request(ctx, http.MethodPost, "/sandboxes/"+name+"/exec", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// WithSandbox creates a sandbox, passes a SandboxSession to fn, and removes
// the sandbox when fn returns — even if fn returns an error.
func (c *Client) WithSandbox(ctx context.Context, name string, opts *CreateSandboxOptions, fn func(session *SandboxSession) error) error {
	_, err := c.CreateSandbox(ctx, name, opts)
	if err != nil {
		return err
	}
	defer c.RemoveSandbox(ctx, name) //nolint:errcheck

	session := &SandboxSession{name: name, client: c}
	return fn(session)
}

// Browser creates a sandboxed headless browser session.
//
// It provisions a sandbox with Chromium (via Playwright) pre-installed and
// returns a BrowserSession. Call Goto, Screenshot, and Evaluate to interact
// with web pages. When done, call Remove (or Close) to tear down the sandbox.
//
//	browser, err := client.Browser(ctx, "my-browser")
//	if err != nil { ... }
//	defer browser.Close()
//	page, _ := browser.Goto(ctx, "https://example.com")
//	fmt.Println(page.Title, page.Links)
func (c *Client) Browser(ctx context.Context, name string, opts ...BrowserOption) (*BrowserSession, error) {
	cfg := browserConfig{memoryMB: 2048}
	for _, o := range opts {
		o(&cfg)
	}

	_, err := c.CreateSandbox(ctx, name, &CreateSandboxOptions{
		Image:    "python:3.12-slim",
		MemoryMB: cfg.memoryMB,
		Profile:  ProfileModerate,
	})
	if err != nil {
		return nil, fmt.Errorf("browser: create sandbox: %w", err)
	}

	// Install Playwright + Chromium (one-time setup).
	setupCmd := []string{"sh", "-c", "pip install -q playwright && playwright install --with-deps chromium"}
	if _, err := c.ExecInSandbox(ctx, name, setupCmd); err != nil {
		// Best-effort cleanup on setup failure.
		_ = c.RemoveSandbox(ctx, name)
		return nil, fmt.Errorf("browser: install playwright: %w", err)
	}

	return &BrowserSession{name: name, client: c}, nil
}

// ReadFile reads a file from a sandbox.
func (c *Client) ReadFile(ctx context.Context, name, path string) (*FileReadResponse, error) {
	var result FileReadResponse
	err := c.request(ctx, http.MethodGet, "/sandboxes/"+name+"/files/"+path, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// WriteFile writes a file to a sandbox.
func (c *Client) WriteFile(ctx context.Context, name, path, content string, encoding string) error {
	if encoding == "" {
		encoding = "utf8"
	}
	body := fileWriteRequest{Content: content, Encoding: encoding}
	var result string
	return c.request(ctx, http.MethodPut, "/sandboxes/"+name+"/files/"+path, body, &result)
}

// DeleteFile deletes a file from a sandbox.
func (c *Client) DeleteFile(ctx context.Context, name, path string) error {
	var result string
	return c.request(ctx, http.MethodDelete, "/sandboxes/"+name+"/files/"+path, nil, &result)
}

// GetSandboxLogs returns audit log entries for a sandbox.
func (c *Client) GetSandboxLogs(ctx context.Context, name string) ([]map[string]interface{}, error) {
	var result []map[string]interface{}
	err := c.request(ctx, http.MethodGet, "/sandboxes/"+name+"/logs", nil, &result)
	return result, err
}

// BatchRun executes multiple commands in parallel.
func (c *Client) BatchRun(ctx context.Context, commands []BatchCommand) (*BatchRunResponse, error) {
	body := batchRunRequest{Commands: commands}
	var result BatchRunResponse
	err := c.request(ctx, http.MethodPost, "/batch/run", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// ListOrchestrations returns all orchestrations.
func (c *Client) ListOrchestrations(ctx context.Context) ([]Orchestration, error) {
	var result []Orchestration
	err := c.request(ctx, http.MethodGet, "/orchestrations", nil, &result)
	return result, err
}

// CreateOrchestration creates a new orchestration.
func (c *Client) CreateOrchestration(ctx context.Context, body CreateOrchestrationRequest) (*Orchestration, error) {
	var result Orchestration
	err := c.request(ctx, http.MethodPost, "/orchestrations", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetOrchestration returns an orchestration by id.
func (c *Client) GetOrchestration(ctx context.Context, id string) (*Orchestration, error) {
	var result Orchestration
	err := c.request(ctx, http.MethodGet, "/orchestrations/"+id, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// SignalOrchestration raises an external event for an orchestration.
func (c *Client) SignalOrchestration(ctx context.Context, id string, body map[string]interface{}) (*Orchestration, error) {
	var result Orchestration
	err := c.request(ctx, http.MethodPost, "/orchestrations/"+id+"/events", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// TerminateOrchestration terminates an orchestration.
func (c *Client) TerminateOrchestration(ctx context.Context, id string, body map[string]interface{}) (*Orchestration, error) {
	var result Orchestration
	err := c.request(ctx, http.MethodPost, "/orchestrations/"+id+"/terminate", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// ListObjects returns all durable objects.
func (c *Client) ListObjects(ctx context.Context) ([]DurableObject, error) {
	var result []DurableObject
	err := c.request(ctx, http.MethodGet, "/objects", nil, &result)
	return result, err
}

// CreateObject creates a new durable object.
func (c *Client) CreateObject(ctx context.Context, body CreateObjectRequest) (*DurableObject, error) {
	var result DurableObject
	err := c.request(ctx, http.MethodPost, "/objects", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetObject returns a durable object by id.
func (c *Client) GetObject(ctx context.Context, id string) (*DurableObject, error) {
	var result DurableObject
	err := c.request(ctx, http.MethodGet, "/objects/"+id, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// ListSchedules returns all schedules.
func (c *Client) ListSchedules(ctx context.Context) ([]Schedule, error) {
	var result []Schedule
	err := c.request(ctx, http.MethodGet, "/schedules", nil, &result)
	return result, err
}

// CreateSchedule creates a new schedule.
func (c *Client) CreateSchedule(ctx context.Context, body CreateScheduleRequest) (*Schedule, error) {
	var result Schedule
	err := c.request(ctx, http.MethodPost, "/schedules", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetSchedule returns a schedule by id.
func (c *Client) GetSchedule(ctx context.Context, id string) (*Schedule, error) {
	var result Schedule
	err := c.request(ctx, http.MethodGet, "/schedules/"+id, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// ExtendTTL extends a sandbox's time-to-live. Returns the new expiry time.
func (c *Client) ExtendTTL(ctx context.Context, name string, by string) (*ExtendTtlResponse, error) {
	body := extendTtlRequest{By: by}
	var result ExtendTtlResponse
	err := c.request(ctx, http.MethodPost, "/sandboxes/"+name+"/extend", body, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// ListSnapshots returns all snapshots.
func (c *Client) ListSnapshots(ctx context.Context) ([]SnapshotMeta, error) {
	var result []SnapshotMeta
	err := c.request(ctx, http.MethodGet, "/snapshots", nil, &result)
	return result, err
}

// TakeSnapshot creates a snapshot of a sandbox.
func (c *Client) TakeSnapshot(ctx context.Context, opts *TakeSnapshotOptions) (*SnapshotMeta, error) {
	var result SnapshotMeta
	err := c.request(ctx, http.MethodPost, "/snapshots", opts, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// GetSnapshot returns info about a snapshot.
func (c *Client) GetSnapshot(ctx context.Context, name string) (*SnapshotMeta, error) {
	var result SnapshotMeta
	err := c.request(ctx, http.MethodGet, "/snapshots/"+name, nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// DeleteSnapshot removes a snapshot.
func (c *Client) DeleteSnapshot(ctx context.Context, name string) error {
	var result string
	return c.request(ctx, http.MethodDelete, "/snapshots/"+name, nil, &result)
}

// RestoreSnapshot restores a sandbox from a snapshot.
func (c *Client) RestoreSnapshot(ctx context.Context, name string) (*SandboxInfo, error) {
	var result SandboxInfo
	err := c.request(ctx, http.MethodPost, "/snapshots/"+name+"/restore", nil, &result)
	if err != nil {
		return nil, err
	}
	return &result, nil
}

// --- internal ---

func (c *Client) applyHeaders(req *http.Request) {
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("User-Agent", "agentkernel-go-sdk/"+sdkVersion)
	if c.apiKey != "" {
		req.Header.Set("Authorization", "Bearer "+c.apiKey)
	}
}

func (c *Client) request(ctx context.Context, method, path string, body interface{}, result interface{}) error {
	var bodyReader io.Reader
	if body != nil {
		jsonBody, err := json.Marshal(body)
		if err != nil {
			return fmt.Errorf("agentkernel: marshal request: %w", err)
		}
		bodyReader = bytes.NewReader(jsonBody)
	}

	req, err := http.NewRequestWithContext(ctx, method, c.baseURL+path, bodyReader)
	if err != nil {
		return fmt.Errorf("agentkernel: create request: %w", err)
	}
	c.applyHeaders(req)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("agentkernel: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode >= 400 {
		return c.handleErrorResponse(resp)
	}

	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("agentkernel: read response: %w", err)
	}

	var apiResp apiResponse[json.RawMessage]
	if err := json.Unmarshal(respBody, &apiResp); err != nil {
		return fmt.Errorf("agentkernel: decode response: %w", err)
	}

	if !apiResp.Success {
		msg := apiResp.Error
		if msg == "" {
			msg = "unknown error"
		}
		return &Error{StatusCode: resp.StatusCode, Message: msg}
	}

	if result != nil && apiResp.Data != nil {
		if err := json.Unmarshal(apiResp.Data, result); err != nil {
			return fmt.Errorf("agentkernel: decode data: %w", err)
		}
	}
	return nil
}

func (c *Client) handleErrorResponse(resp *http.Response) error {
	body, _ := io.ReadAll(resp.Body)
	var errResp struct {
		Error string `json:"error"`
	}
	if json.Unmarshal(body, &errResp) == nil && errResp.Error != "" {
		return errorFromStatus(resp.StatusCode, errResp.Error)
	}
	return errorFromStatus(resp.StatusCode, "")
}
