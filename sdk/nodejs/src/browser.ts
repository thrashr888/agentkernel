/**
 * Browser session for orchestrating headless browsers in sandboxes.
 *
 * Each method generates a self-contained Python/Playwright script,
 * runs it inside the sandbox, and parses the JSON result.
 */

import type { AriaSnapshot, PageResult, RunOutput } from "./types.js";

type RunInSandboxFn = (
  name: string,
  command: string[],
) => Promise<RunOutput>;
type RemoveSandboxFn = (name: string) => Promise<void>;

const GOTO_SCRIPT = `
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
`;

const SCREENSHOT_SCRIPT = `
import asyncio, base64, sys
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
`;

const EVALUATE_SCRIPT = `
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
`;

/** Command to install Playwright + Chromium inside the sandbox. */
export const BROWSER_SETUP_CMD = [
  "sh",
  "-c",
  "pip install -q playwright && playwright install --with-deps chromium",
];

// --- v2 scripts ---

const BROWSER_HEALTH_SCRIPT = `
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
`;

const BROWSER_REQUEST_SCRIPT = `
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
`;

/**
 * A sandboxed headless browser controlled from outside.
 *
 * The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
 * You call high-level methods; the SDK generates and runs scripts internally.
 *
 * @example
 * ```ts
 * await using browser = await client.browser("my-browser");
 * const page = await browser.goto("https://example.com");
 * console.log(page.title, page.links);
 * ```
 */
export class BrowserSession implements AsyncDisposable {
  readonly name: string;
  private _removed = false;
  private _lastUrl: string | null = null;
  private readonly _run: RunInSandboxFn;
  private readonly _remove: RemoveSandboxFn;

  /** @internal */
  constructor(name: string, runFn: RunInSandboxFn, removeFn: RemoveSandboxFn) {
    this.name = name;
    this._run = runFn;
    this._remove = removeFn;
  }

  /** Navigate to a URL and return page data (title, text, links). */
  async goto(url: string): Promise<PageResult> {
    const result = await this._run(this.name, [
      "python3",
      "-c",
      GOTO_SCRIPT,
      url,
    ]);
    this._lastUrl = url;
    return JSON.parse(result.output) as PageResult;
  }

  /** Take a PNG screenshot. Returns a base64-encoded string. */
  async screenshot(url?: string): Promise<string> {
    const target = url ?? this._lastUrl;
    if (!target) {
      throw new Error("No URL specified and no previous goto() call");
    }
    const result = await this._run(this.name, [
      "python3",
      "-c",
      SCREENSHOT_SCRIPT,
      target,
    ]);
    return result.output.trim();
  }

  /** Run a JavaScript expression on a page and return the result. */
  async evaluate(expression: string, url?: string): Promise<unknown> {
    const target = url ?? this._lastUrl;
    if (!target) {
      throw new Error("No URL specified and no previous goto() call");
    }
    const result = await this._run(this.name, [
      "python3",
      "-c",
      EVALUATE_SCRIPT,
      target,
      expression,
    ]);
    return JSON.parse(result.output);
  }

  // --- v2 methods (ARIA snapshots, persistent pages, ref-based interaction) ---

  /** Navigate to a URL and return an ARIA snapshot. */
  async open(url: string, page = "default"): Promise<AriaSnapshot> {
    await this._ensureServer();
    const result = await this._browserRequest(
      "POST",
      `/pages/${page}/goto`,
      JSON.stringify({ url }),
    );
    return JSON.parse(result.output) as AriaSnapshot;
  }

  /** Get the current ARIA snapshot without navigating. */
  async snapshot(page = "default"): Promise<AriaSnapshot> {
    await this._ensureServer();
    const result = await this._browserRequest(
      "GET",
      `/pages/${page}/snapshot`,
    );
    return JSON.parse(result.output) as AriaSnapshot;
  }

  /** Click an element by ref ID or CSS selector. Returns new snapshot. */
  async click(opts: { ref?: string; selector?: string; page?: string }): Promise<AriaSnapshot> {
    const page = opts.page ?? "default";
    await this._ensureServer();
    const body: Record<string, string> = {};
    if (opts.ref) body.ref = opts.ref;
    if (opts.selector) body.selector = opts.selector;
    const result = await this._browserRequest(
      "POST",
      `/pages/${page}/click`,
      JSON.stringify(body),
    );
    const data = JSON.parse(result.output);
    if (data.error) throw new Error(data.error);
    return data as AriaSnapshot;
  }

  /** Fill an input by ref ID or CSS selector. Returns new snapshot. */
  async fill(
    value: string,
    opts: { ref?: string; selector?: string; page?: string },
  ): Promise<AriaSnapshot> {
    const page = opts.page ?? "default";
    await this._ensureServer();
    const body: Record<string, string> = { value };
    if (opts.ref) body.ref = opts.ref;
    if (opts.selector) body.selector = opts.selector;
    const result = await this._browserRequest(
      "POST",
      `/pages/${page}/fill`,
      JSON.stringify(body),
    );
    const data = JSON.parse(result.output);
    if (data.error) throw new Error(data.error);
    return data as AriaSnapshot;
  }

  /** Close a named page. */
  async closePage(page = "default"): Promise<void> {
    await this._ensureServer();
    await this._browserRequest("DELETE", `/pages/${page}`);
  }

  /** List active page names. */
  async listPages(): Promise<string[]> {
    await this._ensureServer();
    const result = await this._browserRequest("GET", "/pages");
    const data = JSON.parse(result.output);
    return data.pages ?? [];
  }

  private _serverStarted = false;

  private async _ensureServer(): Promise<void> {
    if (this._serverStarted) return;
    // Health check
    try {
      const result = await this._run(this.name, [
        "python3", "-c", BROWSER_HEALTH_SCRIPT, "9222",
      ]);
      if (result.output.includes('"status":"ok"') || result.output.includes('"status": "ok"')) {
        this._serverStarted = true;
        return;
      }
    } catch {
      // Server not running, start it
    }
    // Start would require the full server script + ARIA JS — for SDK use,
    // the server is expected to already be running (started via browser_create or MCP)
    throw new Error(
      "Browser server not running. Use browser_create to set up the sandbox, " +
      "or use the MCP browser_open tool which auto-starts the server.",
    );
  }

  private async _browserRequest(
    method: string,
    path: string,
    body?: string,
  ): Promise<RunOutput> {
    const cmd = ["python3", "-c", BROWSER_REQUEST_SCRIPT, "9222", method, path];
    if (body) cmd.push(body);
    return this._run(this.name, cmd);
  }

  // --- v1 methods (backward compatible) ---

  /** Remove the sandbox. Idempotent. */
  async remove(): Promise<void> {
    if (this._removed) return;
    this._removed = true;
    await this._remove(this.name);
  }

  /** Auto-cleanup for `await using`. */
  async [Symbol.asyncDispose](): Promise<void> {
    await this.remove();
  }
}
