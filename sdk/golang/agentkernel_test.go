package agentkernel

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// helper: create a client pointing at a test server
func testClient(handler http.HandlerFunc) (*Client, *httptest.Server) {
	srv := httptest.NewServer(handler)
	client := New(&Options{BaseURL: srv.URL})
	return client, srv
}

// helper: respond with a JSON API response
func jsonOK(w http.ResponseWriter, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	resp := map[string]interface{}{"success": true, "data": data}
	json.NewEncoder(w).Encode(resp)
}

func jsonError(w http.ResponseWriter, status int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	resp := map[string]interface{}{"error": msg}
	json.NewEncoder(w).Encode(resp)
}

func readBody(r *http.Request) map[string]interface{} {
	body, _ := io.ReadAll(r.Body)
	var m map[string]interface{}
	json.Unmarshal(body, &m)
	return m
}

// --- Tests ---

func TestHealth(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/health" {
			t.Fatalf("expected /health, got %s", r.URL.Path)
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	result, err := client.Health(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if result != "ok" {
		t.Fatalf("expected ok, got %s", result)
	}
}

func TestRun(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" {
			t.Fatalf("expected POST, got %s", r.Method)
		}
		if !strings.HasSuffix(r.URL.Path, "/run") {
			t.Fatalf("expected /run, got %s", r.URL.Path)
		}
		body := readBody(r)
		cmd := body["command"].([]interface{})
		if cmd[0] != "echo" || cmd[1] != "hello" {
			t.Fatalf("unexpected command: %v", cmd)
		}
		if body["fast"] != true {
			t.Fatalf("expected fast=true, got %v", body["fast"])
		}
		jsonOK(w, map[string]string{"output": "hello\n"})
	})
	defer srv.Close()

	output, err := client.Run(context.Background(), []string{"echo", "hello"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if output.Output != "hello\n" {
		t.Fatalf("expected 'hello\\n', got %q", output.Output)
	}
}

func TestRunWithOptions(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		body := readBody(r)
		if body["image"] != "python:3.12" {
			t.Fatalf("expected image python:3.12, got %v", body["image"])
		}
		if body["profile"] != "restrictive" {
			t.Fatalf("expected profile restrictive, got %v", body["profile"])
		}
		if body["fast"] != false {
			t.Fatalf("expected fast=false, got %v", body["fast"])
		}
		jsonOK(w, map[string]string{"output": "done"})
	})
	defer srv.Close()

	fast := false
	opts := &RunOptions{Image: "python:3.12", Profile: ProfileRestrictive, Fast: &fast}
	output, err := client.Run(context.Background(), []string{"python", "-c", "print('hi')"}, opts)
	if err != nil {
		t.Fatal(err)
	}
	if output.Output != "done" {
		t.Fatalf("expected 'done', got %q", output.Output)
	}
}

func TestListSandboxes(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		jsonOK(w, []map[string]string{
			{"name": "sb1", "status": "running", "backend": "docker"},
		})
	})
	defer srv.Close()

	sandboxes, err := client.ListSandboxes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(sandboxes) != 1 {
		t.Fatalf("expected 1 sandbox, got %d", len(sandboxes))
	}
	if sandboxes[0].Name != "sb1" {
		t.Fatalf("expected sb1, got %s", sandboxes[0].Name)
	}
	if sandboxes[0].Status != "running" {
		t.Fatalf("expected running, got %s", sandboxes[0].Status)
	}
}

func TestCreateSandbox(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" {
			t.Fatalf("expected POST, got %s", r.Method)
		}
		body := readBody(r)
		if body["name"] != "test-sb" {
			t.Fatalf("expected name test-sb, got %v", body["name"])
		}
		jsonOK(w, map[string]string{"name": "test-sb", "status": "running", "backend": "docker"})
	})
	defer srv.Close()

	sb, err := client.CreateSandbox(context.Background(), "test-sb", nil)
	if err != nil {
		t.Fatal(err)
	}
	if sb.Name != "test-sb" {
		t.Fatalf("expected test-sb, got %s", sb.Name)
	}
}

func TestGetSandbox(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasSuffix(r.URL.Path, "/sandboxes/my-sb") {
			t.Fatalf("expected /sandboxes/my-sb, got %s", r.URL.Path)
		}
		jsonOK(w, map[string]string{"name": "my-sb", "status": "running", "backend": "docker"})
	})
	defer srv.Close()

	sb, err := client.GetSandbox(context.Background(), "my-sb")
	if err != nil {
		t.Fatal(err)
	}
	if sb.Name != "my-sb" {
		t.Fatalf("expected my-sb, got %s", sb.Name)
	}
}

func TestRemoveSandbox(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "DELETE" {
			t.Fatalf("expected DELETE, got %s", r.Method)
		}
		if !strings.HasSuffix(r.URL.Path, "/sandboxes/my-sb") {
			t.Fatalf("expected /sandboxes/my-sb, got %s", r.URL.Path)
		}
		jsonOK(w, "removed")
	})
	defer srv.Close()

	err := client.RemoveSandbox(context.Background(), "my-sb")
	if err != nil {
		t.Fatal(err)
	}
}

func TestExecInSandbox(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" {
			t.Fatalf("expected POST, got %s", r.Method)
		}
		if !strings.HasSuffix(r.URL.Path, "/sandboxes/test-sb/exec") {
			t.Fatalf("expected /sandboxes/test-sb/exec, got %s", r.URL.Path)
		}
		body := readBody(r)
		cmd := body["command"].([]interface{})
		if cmd[0] != "ls" {
			t.Fatalf("expected ls, got %v", cmd[0])
		}
		jsonOK(w, map[string]string{"output": "total 0\n"})
	})
	defer srv.Close()

	output, err := client.ExecInSandbox(context.Background(), "test-sb", []string{"ls", "-la"})
	if err != nil {
		t.Fatal(err)
	}
	if output.Output != "total 0\n" {
		t.Fatalf("expected 'total 0\\n', got %q", output.Output)
	}
}

func TestWithSandbox(t *testing.T) {
	var paths []string
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.Method+" "+r.URL.Path)
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": "result"})
			return
		}
		jsonOK(w, map[string]string{"name": "tmp", "status": "running", "backend": "docker"})
	})
	defer srv.Close()

	var got string
	err := client.WithSandbox(context.Background(), "tmp", nil, func(session *SandboxSession) error {
		output, err := session.Run(context.Background(), []string{"echo", "hi"})
		if err != nil {
			return err
		}
		got = output.Output
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if got != "result" {
		t.Fatalf("expected result, got %q", got)
	}

	// Verify create and delete were called
	hasCreate := false
	hasDelete := false
	for _, p := range paths {
		if p == "POST /sandboxes" {
			hasCreate = true
		}
		if p == "DELETE /sandboxes/tmp" {
			hasDelete = true
		}
	}
	if !hasCreate {
		t.Fatal("expected POST /sandboxes")
	}
	if !hasDelete {
		t.Fatal("expected DELETE /sandboxes/tmp")
	}
}

func TestWithSandboxCleansUpOnError(t *testing.T) {
	deleteCalled := false
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "DELETE" {
			deleteCalled = true
			jsonOK(w, "removed")
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonError(w, 500, "exec failed")
			return
		}
		jsonOK(w, map[string]string{"name": "tmp", "status": "running", "backend": "docker"})
	})
	defer srv.Close()

	err := client.WithSandbox(context.Background(), "tmp", nil, func(session *SandboxSession) error {
		_, err := session.Run(context.Background(), []string{"bad"})
		return err
	})
	if err == nil {
		t.Fatal("expected error")
	}
	if !deleteCalled {
		t.Fatal("sandbox should be removed even on error")
	}
}

func TestAuthError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		jsonError(w, 401, "invalid key")
	})
	defer srv.Close()

	_, err := client.Health(context.Background())
	if err == nil {
		t.Fatal("expected error")
	}
	if !IsAuthError(err) {
		t.Fatalf("expected auth error, got %v", err)
	}
	e := err.(*Error)
	if e.Message != "invalid key" {
		t.Fatalf("expected 'invalid key', got %q", e.Message)
	}
}

func TestValidationError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		jsonError(w, 400, "bad request")
	})
	defer srv.Close()

	_, err := client.Run(context.Background(), []string{""}, nil)
	if err == nil {
		t.Fatal("expected error")
	}
	if !IsValidationError(err) {
		t.Fatalf("expected validation error, got %v", err)
	}
}

func TestNotFoundError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		jsonError(w, 404, "sandbox not found")
	})
	defer srv.Close()

	_, err := client.GetSandbox(context.Background(), "nonexistent")
	if err == nil {
		t.Fatal("expected error")
	}
	if !IsNotFoundError(err) {
		t.Fatalf("expected not found error, got %v", err)
	}
}

func TestServerError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		jsonError(w, 500, "internal failure")
	})
	defer srv.Close()

	_, err := client.Health(context.Background())
	if err == nil {
		t.Fatal("expected error")
	}
	if !IsServerError(err) {
		t.Fatalf("expected server error, got %v", err)
	}
}

func TestUserAgentHeader(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		ua := r.Header.Get("User-Agent")
		if !strings.HasPrefix(ua, "agentkernel-go-sdk/") {
			t.Fatalf("expected agentkernel-go-sdk/ prefix, got %q", ua)
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	client.Health(context.Background())
}

func TestAPIKeyHeader(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		auth := r.Header.Get("Authorization")
		if auth != "Bearer sk-test-123" {
			t.Fatalf("expected Bearer sk-test-123, got %q", auth)
		}
		jsonOK(w, "ok")
	}))
	defer srv.Close()

	client := New(&Options{BaseURL: srv.URL, APIKey: "sk-test-123"})
	client.Health(context.Background())
}

func TestNoAuthHeaderWithoutAPIKey(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if auth := r.Header.Get("Authorization"); auth != "" {
			t.Fatalf("expected no Authorization header, got %q", auth)
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	client.Health(context.Background())
}

func TestAPIFailureResponse(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]interface{}{
			"success": false,
			"error":   "something went wrong",
		})
	})
	defer srv.Close()

	_, err := client.Health(context.Background())
	if err == nil {
		t.Fatal("expected error")
	}
	if !strings.Contains(err.Error(), "something went wrong") {
		t.Fatalf("expected 'something went wrong' in error, got %v", err)
	}
}

func TestStreamParsing(t *testing.T) {
	sse := "event: started\ndata: {\"sandbox\":\"test\"}\n\nevent: output\ndata: {\"content\":\"hello\"}\n\nevent: done\ndata: {\"exit_code\":0}\n\n"
	ch := ParseSSE(strings.NewReader(sse))

	events := make([]StreamEvent, 0)
	for ev := range ch {
		events = append(events, ev)
	}

	if len(events) != 3 {
		t.Fatalf("expected 3 events, got %d", len(events))
	}
	if events[0].Type != "started" {
		t.Fatalf("expected started, got %s", events[0].Type)
	}
	if events[1].Type != "output" {
		t.Fatalf("expected output, got %s", events[1].Type)
	}
	if events[1].Data["content"] != "hello" {
		t.Fatalf("expected hello, got %v", events[1].Data["content"])
	}
	if events[2].Type != "done" {
		t.Fatalf("expected done, got %s", events[2].Type)
	}
}
