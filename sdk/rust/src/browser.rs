//! Browser session for orchestrating headless browsers in sandboxes.
//!
//! Each method generates a self-contained Python/Playwright script,
//! runs it inside the sandbox, and parses the JSON result.

use base64::Engine;

use crate::client::AgentKernel;
use crate::error::{Error, Result};
use crate::types::PageResult;

// ---------------------------------------------------------------------------
// Inline Playwright script templates
// ---------------------------------------------------------------------------

const GOTO_SCRIPT: &str = r#"
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

const SCREENSHOT_SCRIPT: &str = r#"
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
"#;

const EVALUATE_SCRIPT: &str = r#"
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

/// Command to install Playwright + Chromium inside a sandbox.
pub const BROWSER_SETUP_CMD: &[&str] = &[
    "sh",
    "-c",
    "pip install -q playwright && playwright install --with-deps chromium",
];

/// A sandboxed headless browser controlled from outside.
///
/// The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
/// You call high-level methods; the SDK generates and executes scripts internally.
///
/// # Example
///
/// ```no_run
/// # async fn example() -> agentkernel_sdk::Result<()> {
/// let client = agentkernel_sdk::AgentKernel::builder().build()?;
/// let mut browser = client.browser("my-browser", None).await?;
/// let page = browser.goto("https://example.com").await?;
/// println!("{} — {} links", page.title, page.links.len());
/// let png = browser.screenshot(None).await?;
/// browser.remove().await?;
/// # Ok(())
/// # }
/// ```
pub struct BrowserSession {
    /// The sandbox name.
    name: String,
    /// The underlying client.
    client: AgentKernel,
    /// Whether `remove()` has already been called.
    removed: bool,
    /// Last URL visited via `goto()`.
    last_url: Option<String>,
}

impl BrowserSession {
    /// Create a new `BrowserSession`.
    ///
    /// Prefer [`AgentKernel::browser`] which creates the sandbox and installs
    /// Playwright for you.
    pub(crate) fn new(name: String, client: AgentKernel) -> Self {
        Self {
            name,
            client,
            removed: false,
            last_url: None,
        }
    }

    /// The sandbox name backing this browser session.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Navigate to a URL and return page data (title, text, links).
    pub async fn goto(&mut self, url: &str) -> Result<PageResult> {
        let output = self
            .client
            .exec_in_sandbox(&self.name, &["python3", "-c", GOTO_SCRIPT, url], None)
            .await?;
        self.last_url = Some(url.to_string());
        let result: PageResult = serde_json::from_str(&output.output)?;
        Ok(result)
    }

    /// Take a PNG screenshot. Returns the raw PNG bytes.
    ///
    /// If `url` is `None`, re-uses the last URL from [`goto`](Self::goto).
    pub async fn screenshot(&self, url: Option<&str>) -> Result<Vec<u8>> {
        let target = url
            .map(String::from)
            .or_else(|| self.last_url.clone())
            .ok_or_else(|| {
                Error::Validation("No URL specified and no previous goto() call".to_string())
            })?;
        let output = self
            .client
            .exec_in_sandbox(
                &self.name,
                &["python3", "-c", SCREENSHOT_SCRIPT, &target],
                None,
            )
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.output.trim())
            .map_err(|e| Error::Server(format!("base64 decode failed: {e}")))?;
        Ok(bytes)
    }

    /// Run a JavaScript expression on a page and return the result as JSON.
    ///
    /// If `url` is `None`, re-uses the last URL from [`goto`](Self::goto).
    pub async fn evaluate(&self, expression: &str, url: Option<&str>) -> Result<serde_json::Value> {
        let target = url
            .map(String::from)
            .or_else(|| self.last_url.clone())
            .ok_or_else(|| {
                Error::Validation("No URL specified and no previous goto() call".to_string())
            })?;
        let output = self
            .client
            .exec_in_sandbox(
                &self.name,
                &["python3", "-c", EVALUATE_SCRIPT, &target, expression],
                None,
            )
            .await?;
        let value: serde_json::Value = serde_json::from_str(&output.output)?;
        Ok(value)
    }

    /// Remove the underlying sandbox. Idempotent.
    pub async fn remove(&mut self) -> Result<()> {
        if self.removed {
            return Ok(());
        }
        self.removed = true;
        self.client.remove_sandbox(&self.name).await
    }
}
