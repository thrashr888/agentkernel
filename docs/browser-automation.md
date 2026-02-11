
# Browser Automation

Run headless browsers inside sandboxes, orchestrate them from outside. The browser runs in an isolated container — your agent controls it through the agentkernel CLI, SDK, or MCP tools.

## Why Sandboxed Browsers?

AI agents need to browse the web. Running a browser inside an agentkernel sandbox means the agent orchestrates from outside while the browser runs in full isolation:

- **Isolation**: Untrusted pages can't access your host filesystem or network
- **Orchestration**: Control the browser via CLI exec, SDK, or MCP — not inside the sandbox
- **Disposability**: Spin up a browser sandbox, use it, throw it away

## SDK Browser Sessions

Every SDK provides a `BrowserSession` that handles sandbox creation, Playwright installation, and script execution. You call high-level methods — the SDK manages everything internally.

### Python

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    with client.browser("my-browser") as browser:
        page = browser.goto("https://example.com")
        print(page.title)        # "Example Domain"
        print(page.text[:200])   # body text (truncated to 8KB)
        print(page.links)        # [PageLink(text="More information...", href="https://...")]

        png = browser.screenshot()  # raw PNG bytes
        result = browser.evaluate("document.querySelectorAll('h1').length")
    # sandbox auto-removed
```

### TypeScript / Node.js

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

await using browser = await client.browser("my-browser");
const page = await browser.goto("https://example.com");
console.log(page.title, page.links);

const png = await browser.screenshot();   // base64 string
const count = await browser.evaluate("document.querySelectorAll('h1').length");
// sandbox auto-removed when scope exits
```

### Go

```go
client := agentkernel.New()
browser, err := client.Browser(ctx, "my-browser")
if err != nil { log.Fatal(err) }
defer browser.Close()

page, err := browser.Goto(ctx, "https://example.com")
fmt.Println(page.Title, len(page.Links))

png, err := browser.Screenshot(ctx, "")  // reuses last URL
jsResult, err := browser.Evaluate(ctx, "document.title", "")
```

### Rust

```rust
let client = AgentKernel::new(None)?;
let mut browser = client.browser("my-browser", None).await?;

let page = browser.goto("https://example.com").await?;
println!("{} — {} links", page.title, page.links.len());

let png: Vec<u8> = browser.screenshot(None).await?;
let result: serde_json::Value = browser.evaluate("document.title", None).await?;
browser.remove().await?;
```

### Swift

```swift
let client = AgentKernel()
let browser = try await client.browser("my-browser")

let page = try await browser.goto("https://example.com")
print(page.title, page.links.count)

let png: Data = try await browser.screenshot()
let result = try await browser.evaluate("document.title")
try await browser.remove()
```

### MCP Tools

Agents using agentkernel as an MCP server orchestrate browser sandboxes through tool calls:

```
sandbox_create(name="browser", image="python:3.12-slim")
sandbox_exec(name="browser", command=["sh", "-c",
  "pip install playwright && playwright install --with-deps chromium"])
sandbox_exec(name="browser", command=["python3", "-c", "<inline-script>", "https://example.com"])
sandbox_remove(name="browser")
```

### CLI: Create and Exec

For one-off use without an SDK:

```bash
agentkernel create --template playwright my-browser
agentkernel file write my-browser /app/scrape.py < scrape.py
agentkernel exec my-browser -- python3 /app/scrape.py https://example.com
agentkernel remove my-browser
```

## Return Types

All SDKs return the same structured data from `goto()`:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title |
| `url` | string | Final URL (after redirects) |
| `text` | string | Body innerText, truncated to ~8KB |
| `links` | PageLink[] | First 50 `<a href>` links (text + href) |

## Configuration

### agentkernel.toml

```toml
[sandbox]
name = "browser"
base_image = "python:3.12-slim"

[resources]
vcpus = 2
memory_mb = 2048

[security]
profile = "moderate"
network = true
```

### Image Choice

Playwright needs glibc. Use `python:3.12-slim` (Debian-based), not Alpine.

### Memory

Chromium uses ~500-800 MB at idle. Use at least 2048 MB. For multiple tabs or heavy pages, use 4096 MB.

## See Also

- [Playwright example](https://github.com/thrashr888/agentkernel/tree/main/examples/playwright)
- [Playwright Stealth example](https://github.com/thrashr888/agentkernel/tree/main/examples/playwright-stealth)
- [Python SDK](sdk-python.md)
- [MCP Integration](api-mcp.md)
