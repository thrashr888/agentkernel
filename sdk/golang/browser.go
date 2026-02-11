package agentkernel

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
)

// Inline Playwright script templates.
// Each method generates a self-contained Python script that launches Chromium,
// performs one action, prints JSON to stdout, and exits.

const gotoScript = `
import asyncio, json, sys
from playwright.async_api import async_playwright
async def main():
    url = sys.argv[1]
    async with async_playwright() as p:
        b = await p.chromium.launch()
        page = await b.new_page()
        await page.goto(url, timeout=30000)
        title = await page.title()
        url_final = page.url
        text = await page.evaluate("() => document.body.innerText.slice(0, 8000)")
        links = await page.evaluate('''() =>
            Array.from(document.querySelectorAll('a[href]'))
                .slice(0, 50)
                .map(a => ({text: a.textContent.trim(), href: a.href}))
                .filter(l => l.href.startsWith("http"))
        ''')
        print(json.dumps({"title": title, "url": url_final, "text": text, "links": links}))
        await b.close()
asyncio.run(main())
`

const screenshotScript = `
import asyncio, base64, json, sys
from playwright.async_api import async_playwright
async def main():
    url = sys.argv[1]
    async with async_playwright() as p:
        b = await p.chromium.launch()
        page = await b.new_page()
        await page.goto(url, timeout=30000)
        data = await page.screenshot()
        print(base64.b64encode(data).decode())
        await b.close()
asyncio.run(main())
`

const evaluateScript = `
import asyncio, json, sys
from playwright.async_api import async_playwright
async def main():
    url = sys.argv[1]
    expr = sys.argv[2]
    async with async_playwright() as p:
        b = await p.chromium.launch()
        page = await b.new_page()
        await page.goto(url, timeout=30000)
        result = await page.evaluate(expr)
        print(json.dumps(result))
        await b.close()
asyncio.run(main())
`

// v2 scripts — proxy through the in-sandbox browser server on port 9222.

const browserHealthScript = `
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
`

const browserRequestScript = `
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
method = sys.argv[2] if len(sys.argv) > 2 else "GET"
path = sys.argv[3] if len(sys.argv) > 3 else "/health"
body_str = sys.argv[4] if len(sys.argv) > 4 else None
url = f"http://127.0.0.1:{port}{path}"
data = body_str.encode() if body_str else None
req = urllib.request.Request(url, data=data, method=method)
if data:
    req.add_header("Content-Type", "application/json")
try:
    resp = urllib.request.urlopen(req, timeout=60)
    print(resp.read().decode())
except urllib.error.HTTPError as e:
    print(e.read().decode())
    sys.exit(1)
except Exception as e:
    print(json.dumps({"error": str(e)}))
    sys.exit(1)
`

// BrowserSession provides a sandboxed headless browser controlled from outside.
//
// The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
// You call high-level methods; the SDK generates and executes scripts internally.
//
// Use Client.Browser to create a session. Call Remove (or Close) when done to
// tear down the sandbox. BrowserSession implements io.Closer.
type BrowserSession struct {
	name          string
	client        *Client
	removed       bool
	lastURL       string
	serverStarted bool
}

// Name returns the sandbox name backing this browser session.
func (b *BrowserSession) Name() string {
	return b.name
}

// Goto navigates to a URL and returns page data (title, text, links).
func (b *BrowserSession) Goto(ctx context.Context, url string) (*PageResult, error) {
	output, err := b.runScript(ctx, gotoScript, url)
	if err != nil {
		return nil, fmt.Errorf("browser goto: %w", err)
	}
	var result PageResult
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser goto: decode output: %w", err)
	}
	b.lastURL = url
	return &result, nil
}

// Screenshot takes a PNG screenshot of the page at the given URL.
// If url is empty, the last URL from Goto is used.
// Returns the raw PNG bytes.
func (b *BrowserSession) Screenshot(ctx context.Context, url string) ([]byte, error) {
	target := url
	if target == "" {
		target = b.lastURL
	}
	if target == "" {
		return nil, fmt.Errorf("browser screenshot: no URL specified and no previous Goto call")
	}
	output, err := b.runScript(ctx, screenshotScript, target)
	if err != nil {
		return nil, fmt.Errorf("browser screenshot: %w", err)
	}
	data, err := base64.StdEncoding.DecodeString(strings.TrimSpace(output))
	if err != nil {
		return nil, fmt.Errorf("browser screenshot: decode base64: %w", err)
	}
	return data, nil
}

// Evaluate runs a JavaScript expression on the page at the given URL and
// returns the parsed JSON result.
// If url is empty, the last URL from Goto is used.
func (b *BrowserSession) Evaluate(ctx context.Context, expression string, url string) (interface{}, error) {
	target := url
	if target == "" {
		target = b.lastURL
	}
	if target == "" {
		return nil, fmt.Errorf("browser evaluate: no URL specified and no previous Goto call")
	}
	output, err := b.runScript(ctx, evaluateScript, target, expression)
	if err != nil {
		return nil, fmt.Errorf("browser evaluate: %w", err)
	}
	var result interface{}
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser evaluate: decode output: %w", err)
	}
	return result, nil
}

// Remove tears down the sandbox. Idempotent — safe to call multiple times.
func (b *BrowserSession) Remove(ctx context.Context) error {
	if b.removed {
		return nil
	}
	b.removed = true
	return b.client.RemoveSandbox(ctx, b.name)
}

// Close tears down the sandbox. Alias for Remove with a background context.
// Implements io.Closer.
func (b *BrowserSession) Close() error {
	return b.Remove(context.Background())
}

// runScript executes an inline Python script inside the sandbox and returns
// the stdout output.
func (b *BrowserSession) runScript(ctx context.Context, script string, args ...string) (string, error) {
	cmd := append([]string{"python3", "-c", script}, args...)
	out, err := b.client.ExecInSandbox(ctx, b.name, cmd)
	if err != nil {
		return "", err
	}
	return out.Output, nil
}

// --- v2 methods (ARIA snapshots, persistent pages, ref-based interaction) ---

// ensureServer checks that the in-sandbox browser server is running.
func (b *BrowserSession) ensureServer(ctx context.Context) error {
	if b.serverStarted {
		return nil
	}
	output, err := b.runScript(ctx, browserHealthScript, "9222")
	if err == nil && (strings.Contains(output, `"status":"ok"`) || strings.Contains(output, `"status": "ok"`)) {
		b.serverStarted = true
		return nil
	}
	return fmt.Errorf("browser server not running — use browser_create or MCP browser_open to start it")
}

// browserRequest sends an HTTP request to the in-sandbox browser server.
func (b *BrowserSession) browserRequest(ctx context.Context, method, path string, body ...string) (string, error) {
	args := []string{"9222", method, path}
	if len(body) > 0 {
		args = append(args, body[0])
	}
	return b.runScript(ctx, browserRequestScript, args...)
}

// Open navigates to a URL and returns an ARIA snapshot.
func (b *BrowserSession) Open(ctx context.Context, url string, page string) (*AriaSnapshot, error) {
	if page == "" {
		page = "default"
	}
	if err := b.ensureServer(ctx); err != nil {
		return nil, err
	}
	bodyJSON, _ := json.Marshal(map[string]string{"url": url})
	output, err := b.browserRequest(ctx, "POST", "/pages/"+page+"/goto", string(bodyJSON))
	if err != nil {
		return nil, fmt.Errorf("browser open: %w", err)
	}
	var result AriaSnapshot
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser open: decode: %w", err)
	}
	return &result, nil
}

// Snapshot returns the current ARIA snapshot without navigating.
func (b *BrowserSession) Snapshot(ctx context.Context, page string) (*AriaSnapshot, error) {
	if page == "" {
		page = "default"
	}
	if err := b.ensureServer(ctx); err != nil {
		return nil, err
	}
	output, err := b.browserRequest(ctx, "GET", "/pages/"+page+"/snapshot")
	if err != nil {
		return nil, fmt.Errorf("browser snapshot: %w", err)
	}
	var result AriaSnapshot
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser snapshot: decode: %w", err)
	}
	return &result, nil
}

// Click clicks an element by ref ID or CSS selector. Returns a new ARIA snapshot.
func (b *BrowserSession) Click(ctx context.Context, page string, ref string, selector string) (*AriaSnapshot, error) {
	if page == "" {
		page = "default"
	}
	if err := b.ensureServer(ctx); err != nil {
		return nil, err
	}
	body := map[string]string{}
	if ref != "" {
		body["ref"] = ref
	}
	if selector != "" {
		body["selector"] = selector
	}
	bodyJSON, _ := json.Marshal(body)
	output, err := b.browserRequest(ctx, "POST", "/pages/"+page+"/click", string(bodyJSON))
	if err != nil {
		return nil, fmt.Errorf("browser click: %w", err)
	}
	var result AriaSnapshot
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser click: decode: %w", err)
	}
	return &result, nil
}

// Fill fills an input by ref ID or CSS selector. Returns a new ARIA snapshot.
func (b *BrowserSession) Fill(ctx context.Context, value string, page string, ref string, selector string) (*AriaSnapshot, error) {
	if page == "" {
		page = "default"
	}
	if err := b.ensureServer(ctx); err != nil {
		return nil, err
	}
	body := map[string]string{"value": value}
	if ref != "" {
		body["ref"] = ref
	}
	if selector != "" {
		body["selector"] = selector
	}
	bodyJSON, _ := json.Marshal(body)
	output, err := b.browserRequest(ctx, "POST", "/pages/"+page+"/fill", string(bodyJSON))
	if err != nil {
		return nil, fmt.Errorf("browser fill: %w", err)
	}
	var result AriaSnapshot
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser fill: decode: %w", err)
	}
	return &result, nil
}

// ClosePage closes a named page.
func (b *BrowserSession) ClosePage(ctx context.Context, page string) error {
	if page == "" {
		page = "default"
	}
	if err := b.ensureServer(ctx); err != nil {
		return err
	}
	_, err := b.browserRequest(ctx, "DELETE", "/pages/"+page)
	return err
}

// ListPages lists active page names.
func (b *BrowserSession) ListPages(ctx context.Context) ([]string, error) {
	if err := b.ensureServer(ctx); err != nil {
		return nil, err
	}
	output, err := b.browserRequest(ctx, "GET", "/pages")
	if err != nil {
		return nil, fmt.Errorf("browser list pages: %w", err)
	}
	var result struct {
		Pages []string `json:"pages"`
	}
	if err := json.Unmarshal([]byte(output), &result); err != nil {
		return nil, fmt.Errorf("browser list pages: decode: %w", err)
	}
	return result.Pages, nil
}
