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

    // MARK: - Internal

    private func runScript(_ script: String, _ args: String...) async throws -> RunOutput {
        var command = ["python3", "-c", script]
        command.append(contentsOf: args)
        return try await client.execInSandbox(name, command: command)
    }
}
