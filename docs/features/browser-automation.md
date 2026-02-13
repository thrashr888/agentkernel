
# Browser Automation

Run headless browsers inside sandboxes, orchestrate them from outside. The browser runs in an isolated container — your agent controls it through the agentkernel CLI, SDK, or MCP tools.

## Why Sandboxed Browsers?

AI agents need to browse the web. Running a browser inside an agentkernel sandbox means the agent orchestrates from outside while the browser runs in full isolation:

- **Isolation**: Untrusted pages can't access your host filesystem or network
- **Orchestration**: Control the browser via CLI exec, SDK, or MCP — not inside the sandbox
- **Disposability**: Spin up a browser sandbox, use it, throw it away

## ARIA Snapshots

Browser methods return structured ARIA accessibility tree snapshots instead of raw text. The snapshot is a compact YAML representation of the page's accessibility tree with ref IDs on interactive elements:

```yaml
- document "Example Domain":
  - heading "Example Domain" [level=1] [ref=e1]
  - paragraph:
    - text "This domain is for use in illustrative examples."
  - link "More information..." [ref=e2]
    - text "More information..."
```

Ref IDs (`e1`, `e2`, ...) are assigned to interactive elements (links, buttons, inputs). Use them with `click()` and `fill()` to target elements without brittle CSS selectors.

### AriaSnapshot Type

| Field | Type | Description |
|-------|------|-------------|
| `snapshot` | string | ARIA tree as YAML |
| `url` | string | Current page URL |
| `title` | string | Page title |
| `refs` | string[] | Available ref IDs for interactive elements |

## SDK Browser Sessions

Every SDK provides a `BrowserSession` that handles sandbox creation, Playwright installation, and script execution. You call high-level methods — the SDK manages everything internally.

Two sets of methods are available:

- **ARIA methods** — `open()`, `snapshot()`, `click()`, `fill()`, `closePage()`, `listPages()` — use a persistent browser server with ARIA snapshots and ref-based targeting
- **Basic methods** — `goto()`, `screenshot()`, `evaluate()` — launch a fresh Chromium for each call, return raw text/PNG/JSON

### Python

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    with client.browser("my-browser") as browser:
        # ARIA methods — persistent browser, ref-based targeting
        snap = browser.open("https://example.com")
        print(snap.title)     # "Example Domain"
        print(snap.refs)      # ["e1", "e2"]
        print(snap.snapshot)  # ARIA YAML tree

        snap = browser.click(ref="e2")           # click by ref
        snap = browser.fill("query", ref="e3")   # fill by ref
        snap = browser.snapshot()                 # current state

        # Named pages
        snap = browser.open("https://docs.example.com", page="docs")
        pages = browser.list_pages()  # ["default", "docs"]
        browser.close_page("docs")

        # Basic methods — fresh Chromium per call
        page = browser.goto("https://example.com")
        print(page.title, page.text[:200], page.links)
        png = browser.screenshot()
        result = browser.evaluate("document.querySelectorAll('h1').length")
    # sandbox auto-removed
```

### TypeScript / Node.js

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();
await using browser = await client.browser("my-browser");

// ARIA methods
const snap = await browser.open("https://example.com");
console.log(snap.title, snap.refs);

const after = await browser.click({ ref: "e2" });
const filled = await browser.fill("query", { ref: "e3" });
const current = await browser.snapshot();

const pages = await browser.listPages();
await browser.closePage("docs");

// Basic methods
const page = await browser.goto("https://example.com");
const png = await browser.screenshot();
const count = await browser.evaluate("document.querySelectorAll('h1').length");
```

### Go

```go
client := agentkernel.New()
browser, err := client.Browser(ctx, "my-browser")
if err != nil { log.Fatal(err) }
defer browser.Close()

// ARIA methods
snap, err := browser.Open(ctx, "https://example.com", "default")
fmt.Println(snap.Title, snap.Refs)

snap, err = browser.Click(ctx, "default", "e2", "")
snap, err = browser.Fill(ctx, "query", "default", "e3", "")
pages, err := browser.ListPages(ctx)

// Basic methods
page, err := browser.Goto(ctx, "https://example.com")
fmt.Println(page.Title, len(page.Links))
png, err := browser.Screenshot(ctx, "")
jsResult, err := browser.Evaluate(ctx, "document.title", "")
```

### Rust

```rust
let client = AgentKernel::new(None)?;
let mut browser = client.browser("my-browser", None).await?;

// ARIA methods
let snap = browser.open("https://example.com", None).await?;
println!("{} — refs: {:?}", snap.title, snap.refs);

let snap = browser.click(None, Some("e2"), None).await?;
let snap = browser.fill("query", None, Some("e3"), None).await?;
let pages = browser.list_pages().await?;

// Basic methods
let page = browser.goto("https://example.com").await?;
let png: Vec<u8> = browser.screenshot(None).await?;
let result: serde_json::Value = browser.evaluate("document.title", None).await?;
browser.remove().await?;
```

### Swift

```swift
let client = AgentKernel()
let browser = try await client.browser("my-browser")

// ARIA methods
let snap = try await browser.open("https://example.com")
print(snap.title, snap.refs)

let after = try await browser.click(ref: "e2")
let filled = try await browser.fill("query", ref: "e3")
let pages = try await browser.listPages()

// Basic methods
let page = try await browser.goto("https://example.com")
let png: Data = try await browser.screenshot()
let result = try await browser.evaluate("document.title")
try await browser.remove()
```

## MCP Tools

Agents using agentkernel as an MCP server control browsers through tool calls. The browser server starts automatically on first use.

```
browser_open(name="my-browser", url="https://example.com")
→ ARIA snapshot with ref IDs

browser_click(name="my-browser", ref="e2")
→ New ARIA snapshot after click

browser_fill(name="my-browser", ref="e3", value="search query")
→ New ARIA snapshot after fill

browser_snapshot(name="my-browser")
→ Current ARIA snapshot

browser_events(name="my-browser", offset=0, limit=50)
→ Sequenced browser events

browser_close(name="my-browser", page="default")
→ Closes a named page
```

See [MCP Integration](../api/mcp.md) for full tool definitions.

## HTTP API

Browser endpoints live under `/sandboxes/{name}/browser/`:

```
POST   /sandboxes/{name}/browser/start                   # Start browser server
GET    /sandboxes/{name}/browser/pages                    # List pages
POST   /sandboxes/{name}/browser/pages                    # Create page
DELETE /sandboxes/{name}/browser/pages/{page}              # Close page
POST   /sandboxes/{name}/browser/pages/{page}/goto         # Navigate
GET    /sandboxes/{name}/browser/pages/{page}/snapshot      # ARIA snapshot
POST   /sandboxes/{name}/browser/pages/{page}/click        # Click element
POST   /sandboxes/{name}/browser/pages/{page}/fill         # Fill input
POST   /sandboxes/{name}/browser/pages/{page}/screenshot   # PNG screenshot
POST   /sandboxes/{name}/browser/pages/{page}/evaluate     # Run JavaScript
GET    /sandboxes/{name}/browser/pages/{page}/content      # Raw page content
GET    /sandboxes/{name}/browser/events                    # Event stream
```

See [HTTP API](../api/http.md) for full endpoint documentation.

## Event Stream

Browser events are sequenced for debugging and context recovery:

```json
[
  {"seq": 1, "type": "page.navigated", "page": "default", "ts": "2026-02-10T12:00:00Z"},
  {"seq": 2, "type": "page.clicked",   "page": "default", "ts": "2026-02-10T12:00:01Z"}
]
```

Use the `offset` parameter to resume from a known position — useful for agents recovering context after compaction.

| Field | Type | Description |
|-------|------|-------------|
| `seq` | integer | Monotonic sequence number |
| `type` | string | Event type (e.g. "page.navigated", "page.clicked") |
| `page` | string | Page name |
| `ts` | string | ISO 8601 timestamp |

## Basic Method Return Types

The `goto()` method returns a `PageResult`:

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Page title |
| `url` | string | Final URL (after redirects) |
| `text` | string | Body innerText, truncated to ~8KB |
| `links` | PageLink[] | First 50 `<a href>` links (text + href) |

## CLI: Create and Exec

For one-off use without an SDK:

```bash
agentkernel sandbox create --template playwright my-browser
agentkernel file write my-browser /app/scrape.py < scrape.py
agentkernel exec my-browser -- python3 /app/scrape.py https://example.com
agentkernel sandbox remove my-browser
```

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
- [Python SDK](../sdks/python.md)
- [MCP Integration](../api/mcp.md)
- [HTTP API](../api/http.md)
