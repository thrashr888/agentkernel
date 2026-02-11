package agentkernel

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"strings"
	"testing"
)

func TestBrowserCreatesAndSetsSandbox(t *testing.T) {
	var calls []string
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		calls = append(calls, r.Method+" "+r.URL.Path)
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			body := readBody(r)
			if body["image"] != "python:3.12-slim" {
				t.Fatalf("expected image python:3.12-slim, got %v", body["image"])
			}
			if body["profile"] != "moderate" {
				t.Fatalf("expected profile moderate, got %v", body["profile"])
			}
			mem, ok := body["memory_mb"].(float64)
			if !ok || int(mem) != 2048 {
				t.Fatalf("expected memory_mb 2048, got %v", body["memory_mb"])
			}
			jsonOK(w, map[string]string{"name": "br", "status": "running", "backend": "docker"})
			return
		}
		if r.Method == "POST" && strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": "setup done"})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	if browser.Name() != "br" {
		t.Fatalf("expected name br, got %s", browser.Name())
	}

	// Verify create + exec (setup) were called
	hasCreate := false
	hasExec := false
	for _, c := range calls {
		if c == "POST /sandboxes" {
			hasCreate = true
		}
		if c == "POST /sandboxes/br/exec" {
			hasExec = true
		}
	}
	if !hasCreate {
		t.Fatal("expected POST /sandboxes")
	}
	if !hasExec {
		t.Fatal("expected POST /sandboxes/br/exec for playwright setup")
	}
}

func TestBrowserWithMemoryOption(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			body := readBody(r)
			mem, ok := body["memory_mb"].(float64)
			if !ok || int(mem) != 4096 {
				t.Fatalf("expected memory_mb 4096, got %v", body["memory_mb"])
			}
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": ""})
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br", WithMemoryMB(4096))
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()
}

func TestBrowserGoto(t *testing.T) {
	pageData := PageResult{
		Title: "Example Domain",
		URL:   "https://example.com/",
		Text:  "This domain is for use in examples.",
		Links: []PageLink{{Text: "More info", Href: "https://iana.org"}},
	}
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			raw, _ := json.Marshal(pageData)
			jsonOK(w, map[string]string{"output": string(raw)})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	page, err := browser.Goto(context.Background(), "https://example.com")
	if err != nil {
		t.Fatal(err)
	}
	if page.Title != "Example Domain" {
		t.Fatalf("expected title 'Example Domain', got %q", page.Title)
	}
	if page.URL != "https://example.com/" {
		t.Fatalf("expected url 'https://example.com/', got %q", page.URL)
	}
	if len(page.Links) != 1 {
		t.Fatalf("expected 1 link, got %d", len(page.Links))
	}
	if page.Links[0].Href != "https://iana.org" {
		t.Fatalf("expected link href 'https://iana.org', got %q", page.Links[0].Href)
	}
}

func TestBrowserScreenshot(t *testing.T) {
	pngBytes := []byte{0x89, 0x50, 0x4E, 0x47} // PNG magic bytes
	encoded := base64.StdEncoding.EncodeToString(pngBytes)

	execCount := 0
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			execCount++
			if execCount == 1 {
				// setup
				jsonOK(w, map[string]string{"output": ""})
				return
			}
			jsonOK(w, map[string]string{"output": encoded})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	data, err := browser.Screenshot(context.Background(), "https://example.com")
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != len(pngBytes) {
		t.Fatalf("expected %d bytes, got %d", len(pngBytes), len(data))
	}
	if data[0] != 0x89 || data[1] != 0x50 {
		t.Fatal("unexpected PNG data")
	}
}

func TestBrowserScreenshotUsesLastURL(t *testing.T) {
	gotoData := PageResult{Title: "T", URL: "https://example.com/", Text: "t", Links: nil}
	pngBytes := []byte{0x89, 0x50}
	encoded := base64.StdEncoding.EncodeToString(pngBytes)

	execCount := 0
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			execCount++
			if execCount == 1 {
				// setup
				jsonOK(w, map[string]string{"output": ""})
				return
			}
			if execCount == 2 {
				// goto
				raw, _ := json.Marshal(gotoData)
				jsonOK(w, map[string]string{"output": string(raw)})
				return
			}
			// screenshot
			jsonOK(w, map[string]string{"output": encoded})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	_, err = browser.Goto(context.Background(), "https://example.com")
	if err != nil {
		t.Fatal(err)
	}

	// Screenshot with empty URL should use last Goto URL
	data, err := browser.Screenshot(context.Background(), "")
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != 2 {
		t.Fatalf("expected 2 bytes, got %d", len(data))
	}
}

func TestBrowserScreenshotNoURLError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": ""})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	_, err = browser.Screenshot(context.Background(), "")
	if err == nil {
		t.Fatal("expected error when no URL specified and no previous Goto")
	}
	if !strings.Contains(err.Error(), "no URL specified") {
		t.Fatalf("expected 'no URL specified' in error, got %v", err)
	}
}

func TestBrowserEvaluate(t *testing.T) {
	execCount := 0
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			execCount++
			if execCount == 1 {
				// setup
				jsonOK(w, map[string]string{"output": ""})
				return
			}
			// evaluate result
			jsonOK(w, map[string]string{"output": `{"count":42}`})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	result, err := browser.Evaluate(context.Background(), "document.querySelectorAll('a').length", "https://example.com")
	if err != nil {
		t.Fatal(err)
	}
	m, ok := result.(map[string]interface{})
	if !ok {
		t.Fatalf("expected map, got %T", result)
	}
	if m["count"] != float64(42) {
		t.Fatalf("expected count=42, got %v", m["count"])
	}
}

func TestBrowserEvaluateNoURLError(t *testing.T) {
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": ""})
			return
		}
		if r.Method == "DELETE" {
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}
	defer browser.Close()

	_, err = browser.Evaluate(context.Background(), "1+1", "")
	if err == nil {
		t.Fatal("expected error when no URL specified and no previous Goto")
	}
}

func TestBrowserRemoveIdempotent(t *testing.T) {
	deleteCount := 0
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			jsonOK(w, map[string]string{"output": ""})
			return
		}
		if r.Method == "DELETE" {
			deleteCount++
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	browser, err := client.Browser(context.Background(), "br")
	if err != nil {
		t.Fatal(err)
	}

	err = browser.Remove(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	// Second call should be a no-op
	err = browser.Remove(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	if deleteCount != 1 {
		t.Fatalf("expected exactly 1 delete call, got %d", deleteCount)
	}
}

func TestBrowserSetupFailureCleansUp(t *testing.T) {
	deleteCalled := false
	execCount := 0
	client, srv := testClient(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "POST" && r.URL.Path == "/sandboxes" {
			jsonOK(w, map[string]string{"name": "br", "status": "running"})
			return
		}
		if strings.HasSuffix(r.URL.Path, "/exec") {
			execCount++
			// Fail the setup command
			jsonError(w, 500, "pip install failed")
			return
		}
		if r.Method == "DELETE" {
			deleteCalled = true
			jsonOK(w, "removed")
			return
		}
		jsonOK(w, "ok")
	})
	defer srv.Close()

	_, err := client.Browser(context.Background(), "br")
	if err == nil {
		t.Fatal("expected error from failed setup")
	}
	if !deleteCalled {
		t.Fatal("sandbox should be cleaned up on setup failure")
	}
}
