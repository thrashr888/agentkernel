//! Browser session for orchestrating headless browsers in sandboxes.
//!
//! Each method generates a self-contained Python/Playwright script,
//! runs it inside the sandbox, and parses the JSON result.

use base64::Engine;

use crate::client::AgentKernel;
use crate::error::{Error, Result};
use crate::types::{AriaSnapshot, PageResult};

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

// -- v2 scripts: proxy through in-sandbox browser server on port 9222 --

const BROWSER_HEALTH_SCRIPT: &str = r#"
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
"#;

const BROWSER_REQUEST_SCRIPT: &str = r#"
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
"#;

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
    /// Whether the v2 browser server has been confirmed running.
    server_started: bool,
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
            server_started: false,
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

    // -- v2 methods (ARIA snapshots, persistent pages, ref-based interaction) --

    /// Check that the in-sandbox browser server is running.
    async fn ensure_server(&mut self) -> Result<()> {
        if self.server_started {
            return Ok(());
        }
        let output = self
            .client
            .exec_in_sandbox(
                &self.name,
                &["python3", "-c", BROWSER_HEALTH_SCRIPT, "9222"],
                None,
            )
            .await;
        if let Ok(out) = output {
            if out.output.contains("\"status\":\"ok\"") || out.output.contains("\"status\": \"ok\"")
            {
                self.server_started = true;
                return Ok(());
            }
        }
        Err(Error::Server(
            "Browser server not running — use browser_create or MCP browser_open to start it"
                .to_string(),
        ))
    }

    /// Send a request to the in-sandbox browser server.
    async fn browser_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String> {
        let mut cmd = vec![
            "python3",
            "-c",
            BROWSER_REQUEST_SCRIPT,
            "9222",
            method,
            path,
        ];
        if let Some(b) = body {
            cmd.push(b);
        }
        let output = self.client.exec_in_sandbox(&self.name, &cmd, None).await?;
        Ok(output.output)
    }

    /// Navigate to a URL and return an ARIA snapshot.
    pub async fn open(&mut self, url: &str, page: Option<&str>) -> Result<AriaSnapshot> {
        let page = page.unwrap_or("default");
        self.ensure_server().await?;
        let body = serde_json::json!({ "url": url }).to_string();
        let path = format!("/pages/{page}/goto");
        let output = self.browser_request("POST", &path, Some(&body)).await?;
        let result: AriaSnapshot = serde_json::from_str(&output)?;
        Ok(result)
    }

    /// Get the current ARIA snapshot without navigating.
    pub async fn snapshot(&mut self, page: Option<&str>) -> Result<AriaSnapshot> {
        let page = page.unwrap_or("default");
        self.ensure_server().await?;
        let path = format!("/pages/{page}/snapshot");
        let output = self.browser_request("GET", &path, None).await?;
        let result: AriaSnapshot = serde_json::from_str(&output)?;
        Ok(result)
    }

    /// Click an element by ref ID or CSS selector. Returns a new ARIA snapshot.
    pub async fn click(
        &mut self,
        page: Option<&str>,
        ref_id: Option<&str>,
        selector: Option<&str>,
    ) -> Result<AriaSnapshot> {
        let page = page.unwrap_or("default");
        self.ensure_server().await?;
        let mut body = serde_json::Map::new();
        if let Some(r) = ref_id {
            body.insert("ref".to_string(), serde_json::Value::String(r.to_string()));
        }
        if let Some(s) = selector {
            body.insert(
                "selector".to_string(),
                serde_json::Value::String(s.to_string()),
            );
        }
        let body_str = serde_json::Value::Object(body).to_string();
        let path = format!("/pages/{page}/click");
        let output = self.browser_request("POST", &path, Some(&body_str)).await?;
        let result: AriaSnapshot = serde_json::from_str(&output)?;
        Ok(result)
    }

    /// Fill an input by ref ID or CSS selector. Returns a new ARIA snapshot.
    pub async fn fill(
        &mut self,
        value: &str,
        page: Option<&str>,
        ref_id: Option<&str>,
        selector: Option<&str>,
    ) -> Result<AriaSnapshot> {
        let page = page.unwrap_or("default");
        self.ensure_server().await?;
        let mut body = serde_json::Map::new();
        body.insert(
            "value".to_string(),
            serde_json::Value::String(value.to_string()),
        );
        if let Some(r) = ref_id {
            body.insert("ref".to_string(), serde_json::Value::String(r.to_string()));
        }
        if let Some(s) = selector {
            body.insert(
                "selector".to_string(),
                serde_json::Value::String(s.to_string()),
            );
        }
        let body_str = serde_json::Value::Object(body).to_string();
        let path = format!("/pages/{page}/fill");
        let output = self.browser_request("POST", &path, Some(&body_str)).await?;
        let result: AriaSnapshot = serde_json::from_str(&output)?;
        Ok(result)
    }

    /// Close a named page.
    pub async fn close_page(&mut self, page: Option<&str>) -> Result<()> {
        let page = page.unwrap_or("default");
        self.ensure_server().await?;
        let path = format!("/pages/{page}");
        self.browser_request("DELETE", &path, None).await?;
        Ok(())
    }

    /// List active page names.
    pub async fn list_pages(&mut self) -> Result<Vec<String>> {
        self.ensure_server().await?;
        let output = self.browser_request("GET", "/pages", None).await?;
        let value: serde_json::Value = serde_json::from_str(&output)?;
        let pages = value["pages"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(pages)
    }
}
