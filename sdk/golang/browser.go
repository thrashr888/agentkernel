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

// BrowserSession provides a sandboxed headless browser controlled from outside.
//
// The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
// You call high-level methods; the SDK generates and executes scripts internally.
//
// Use Client.Browser to create a session. Call Remove (or Close) when done to
// tear down the sandbox. BrowserSession implements io.Closer.
type BrowserSession struct {
	name    string
	client  *Client
	removed bool
	lastURL string
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
