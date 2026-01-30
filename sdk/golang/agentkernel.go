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
	sdkVersion     = "0.1.0"
	defaultBaseURL = "http://localhost:8880"
	defaultTimeout = 30 * time.Second
)

// Options configures the agentkernel client.
type Options struct {
	// BaseURL is the agentkernel server URL. Default: http://localhost:8880
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
