//! Inline Playwright scripts for browser tools.
//!
//! Shared by MCP tools and (future) HTTP API browser endpoints.
//! Each script is a self-contained Python program that launches Chromium,
//! performs one action, prints output to stdout, and exits.

/// Default Docker image for browser sandboxes.
pub const BROWSER_IMAGE: &str = "python:3.12-slim";

/// Default memory allocation (MB) for browser sandboxes.
pub const BROWSER_MEMORY_MB: u64 = 2048;

/// Command to install Playwright + Chromium inside the sandbox.
pub const BROWSER_SETUP_CMD: &[&str] = &[
    "sh",
    "-c",
    "pip install -q playwright && playwright install --with-deps chromium",
];

/// Navigate to URL, extract title/url/text(8KB)/links(50).
/// Args: sys.argv[1] = url
/// Output: JSON `{"title", "url", "text", "links"}`
pub const GOTO_SCRIPT: &str = r#"
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
"#;

/// Take a full-page screenshot.
/// Args: sys.argv[1] = url
/// Output: base64-encoded PNG
pub const SCREENSHOT_SCRIPT: &str = r#"
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
"#;

/// Evaluate a JavaScript expression on a page.
/// Args: sys.argv[1] = url, sys.argv[2] = JS expression
/// Output: JSON result
pub const EVALUATE_SCRIPT: &str = r#"
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
"#;
