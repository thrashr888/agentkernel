/**
 * Browser session for orchestrating headless browsers in sandboxes.
 *
 * Each method generates a self-contained Python/Playwright script,
 * runs it inside the sandbox, and parses the JSON result.
 */

import type { PageResult, RunOutput } from "./types.js";

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
