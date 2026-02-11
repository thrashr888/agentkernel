import Foundation

// MARK: - Inline Playwright Scripts

/// Each method generates a self-contained Python script that launches Chromium,
/// performs one action, prints JSON to stdout, and exits.

private let gotoScript = """
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
"""

private let screenshotScript = """
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
"""

private let evaluateScript = """
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
"""

/// The shell command used to install Playwright and Chromium inside the sandbox.
private let setupCommand = ["sh", "-c", "pip install -q playwright && playwright install --with-deps chromium"]

// -- v2 scripts: proxy through in-sandbox browser server on port 9222 --

private let browserHealthScript = """
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
"""

private let browserRequestScript = """
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
"""

// MARK: - BrowserSession

/// A sandboxed headless browser controlled from outside.
///
/// The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
/// You call high-level methods; the SDK generates and executes scripts internally.
///
/// ```swift
/// let browser = try await client.browser("my-browser")
/// let page = try await browser.goto("https://example.com")
/// print(page.title, page.links)
/// let png = try await browser.screenshot()
/// try await browser.remove()
/// ```
public final class BrowserSession: @unchecked Sendable {
    /// The sandbox name backing this browser session.
    public let name: String

    private let client: AgentKernel
    private var removed = false
    private var lastUrl: String?
    private var serverStarted = false

    init(name: String, client: AgentKernel) {
        self.name = name
        self.client = client
    }

    /// Navigate to a URL and return page data (title, text, links).
    ///
    /// - Parameter url: The URL to navigate to.
    /// - Returns: A ``PageResult`` containing the page title, final URL, body text, and links.
    public func goto(_ url: String) async throws -> PageResult {
        let result = try await runScript(gotoScript, url)
        let data = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            .data(using: .utf8) ?? Data()
        let page = try JSONDecoder().decode(PageResult.self, from: data)
        lastUrl = url
        return page
    }

    /// Take a PNG screenshot of a web page.
    ///
    /// - Parameter url: The URL to screenshot. Uses the last visited URL if `nil`.
    /// - Returns: Raw PNG image data.
    public func screenshot(_ url: String? = nil) async throws -> Data {
        let target = url ?? lastUrl
        guard let target else {
            throw AgentKernelError.validation("No URL specified and no previous goto() call")
        }
        let result = try await runScript(screenshotScript, target)
        let base64String = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let imageData = Data(base64Encoded: base64String) else {
            throw AgentKernelError.server("Failed to decode base64 screenshot data")
        }
        return imageData
    }

    /// Run a JavaScript expression on a page and return the result.
    ///
    /// - Parameters:
    ///   - expression: The JavaScript expression to evaluate.
    ///   - url: The URL to navigate to before evaluating. Uses the last visited URL if `nil`.
    /// - Returns: The parsed JSON result. Use `JSONSerialization` for dynamic types.
    public func evaluate(_ expression: String, url: String? = nil) async throws -> Any {
        let target = url ?? lastUrl
        guard let target else {
            throw AgentKernelError.validation("No URL specified and no previous goto() call")
        }
        let result = try await runScript(evaluateScript, target, expression)
        let data = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            .data(using: .utf8) ?? Data()
        let json = try JSONSerialization.jsonObject(with: data)
        return json
    }

    /// Remove the underlying sandbox. Idempotent.
    public func remove() async throws {
        guard !removed else { return }
        removed = true
        try await client.removeSandbox(name)
    }

    // MARK: - v2 Methods (ARIA snapshots, persistent pages, ref-based interaction)

    /// Navigate to a URL and return an ARIA snapshot.
    ///
    /// - Parameters:
    ///   - url: The URL to navigate to.
    ///   - page: The page name (default: "default").
    /// - Returns: An ``AriaSnapshot`` with the ARIA tree, URL, title, and ref IDs.
    public func open(_ url: String, page: String = "default") async throws -> AriaSnapshot {
        try await ensureServer()
        let body = try JSONSerialization.data(withJSONObject: ["url": url])
        let bodyStr = String(data: body, encoding: .utf8) ?? "{}"
        let output = try await browserRequest("POST", path: "/pages/\(page)/goto", body: bodyStr)
        let data = output.data(using: .utf8) ?? Data()
        return try JSONDecoder().decode(AriaSnapshot.self, from: data)
    }

    /// Get the current ARIA snapshot without navigating.
    ///
    /// - Parameter page: The page name (default: "default").
    /// - Returns: An ``AriaSnapshot``.
    public func snapshot(page: String = "default") async throws -> AriaSnapshot {
        try await ensureServer()
        let output = try await browserRequest("GET", path: "/pages/\(page)/snapshot")
        let data = output.data(using: .utf8) ?? Data()
        return try JSONDecoder().decode(AriaSnapshot.self, from: data)
    }

    /// Click an element by ref ID or CSS selector.
    ///
    /// - Parameters:
    ///   - ref: The ref ID to click (e.g. "e1").
    ///   - selector: A CSS selector to click.
    ///   - page: The page name (default: "default").
    /// - Returns: A new ``AriaSnapshot`` after clicking.
    public func click(ref: String? = nil, selector: String? = nil, page: String = "default") async throws -> AriaSnapshot {
        try await ensureServer()
        var bodyDict: [String: String] = [:]
        if let ref { bodyDict["ref"] = ref }
        if let selector { bodyDict["selector"] = selector }
        let body = try JSONSerialization.data(withJSONObject: bodyDict)
        let bodyStr = String(data: body, encoding: .utf8) ?? "{}"
        let output = try await browserRequest("POST", path: "/pages/\(page)/click", body: bodyStr)
        let data = output.data(using: .utf8) ?? Data()
        return try JSONDecoder().decode(AriaSnapshot.self, from: data)
    }

    /// Fill an input by ref ID or CSS selector.
    ///
    /// - Parameters:
    ///   - value: The value to type into the input.
    ///   - ref: The ref ID of the input.
    ///   - selector: A CSS selector for the input.
    ///   - page: The page name (default: "default").
    /// - Returns: A new ``AriaSnapshot`` after filling.
    public func fill(_ value: String, ref: String? = nil, selector: String? = nil, page: String = "default") async throws -> AriaSnapshot {
        try await ensureServer()
        var bodyDict: [String: String] = ["value": value]
        if let ref { bodyDict["ref"] = ref }
        if let selector { bodyDict["selector"] = selector }
        let body = try JSONSerialization.data(withJSONObject: bodyDict)
        let bodyStr = String(data: body, encoding: .utf8) ?? "{}"
        let output = try await browserRequest("POST", path: "/pages/\(page)/fill", body: bodyStr)
        let data = output.data(using: .utf8) ?? Data()
        return try JSONDecoder().decode(AriaSnapshot.self, from: data)
    }

    /// Close a named page.
    ///
    /// - Parameter page: The page name to close (default: "default").
    public func closePage(_ page: String = "default") async throws {
        try await ensureServer()
        _ = try await browserRequest("DELETE", path: "/pages/\(page)")
    }

    /// List active page names.
    ///
    /// - Returns: An array of page name strings.
    public func listPages() async throws -> [String] {
        try await ensureServer()
        let output = try await browserRequest("GET", path: "/pages")
        let data = output.data(using: .utf8) ?? Data()
        let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        return json?["pages"] as? [String] ?? []
    }

    // MARK: - Internal

    private func runScript(_ script: String, _ args: String...) async throws -> RunOutput {
        var command = ["python3", "-c", script]
        command.append(contentsOf: args)
        return try await client.execInSandbox(name, command: command)
    }

    private func ensureServer() async throws {
        guard !serverStarted else { return }
        do {
            let result = try await client.execInSandbox(name, command: ["python3", "-c", browserHealthScript, "9222"])
            if result.output.contains("\"status\":\"ok\"") || result.output.contains("\"status\": \"ok\"") {
                serverStarted = true
                return
            }
        } catch {
            // Server not running
        }
        throw AgentKernelError.server("Browser server not running — use browser_create or MCP browser_open to start it")
    }

    private func browserRequest(_ method: String, path: String, body: String? = nil) async throws -> String {
        var command = ["python3", "-c", browserRequestScript, "9222", method, path]
        if let body { command.append(body) }
        let result = try await client.execInSandbox(name, command: command)
        return result.output
    }
}
