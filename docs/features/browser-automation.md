
# Browser Automation

Run headless browsers inside sandboxes, orchestrate them from outside. The browser runs in an isolated container — your agent controls it through the agentkernel CLI, SDK, or MCP tools.

## ARIA Snapshots

Browser methods return structured ARIA accessibility tree snapshots with ref IDs on interactive elements:

```yaml
- document "Example Domain":
  - heading "Example Domain" [level=1] [ref=e1]
  - paragraph:
    - text "This domain is for use in illustrative examples."
  - link "More information..." [ref=e2]
```

Ref IDs (`e1`, `e2`, ...) target elements without brittle CSS selectors. Use them with `click()` and `fill()`.

| Field | Type | Description |
|-------|------|-------------|
| `snapshot` | string | ARIA tree as YAML |
| `url` | string | Current page URL |
| `title` | string | Page title |
| `refs` | string[] | Ref IDs for interactive elements |

## SDK Browser Sessions

Every SDK provides a `BrowserSession` with two method sets:

- **ARIA methods** — `open()`, `snapshot()`, `click()`, `fill()`, `closePage()`, `listPages()` — persistent browser with ref-based targeting
- **Basic methods** — `goto()`, `screenshot()`, `evaluate()` — fresh Chromium per call, returns raw text/PNG/JSON

### Python

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    with client.browser("my-browser") as browser:
        # ARIA methods — persistent browser, ref-based targeting
        snap = browser.open("https://example.com")
        print(snap.title, snap.refs)

        snap = browser.click(ref="e2")
        snap = browser.fill("query", ref="e3")

        # Named pages
        snap = browser.open("https://docs.example.com", page="docs")
        pages = browser.list_pages()
        browser.close_page("docs")

        # Basic methods — fresh Chromium per call
        page = browser.goto("https://example.com")
        png = browser.screenshot()
        result = browser.evaluate("document.querySelectorAll('h1').length")
```

### TypeScript

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();
await using browser = await client.browser("my-browser");

const snap = await browser.open("https://example.com");
console.log(snap.title, snap.refs);

await browser.click({ ref: "e2" });
await browser.fill("query", { ref: "e3" });

const page = await browser.goto("https://example.com");
const png = await browser.screenshot();
```

See [SDK Reference](../sdks/index.md) for Go, Rust, and Swift examples.

## MCP Tools

Agents using agentkernel as an MCP server control browsers through tool calls:

```
browser_open(name="my-browser", url="https://example.com")  → ARIA snapshot
browser_click(name="my-browser", ref="e2")                   → ARIA snapshot
browser_fill(name="my-browser", ref="e3", value="query")     → ARIA snapshot
browser_snapshot(name="my-browser")                           → current state
browser_events(name="my-browser", offset=0, limit=50)         → event stream
browser_close(name="my-browser", page="default")              → closes page
```

See [MCP Integration](../api/mcp.md) for full tool definitions.

## HTTP API

Browser endpoints live under `/sandboxes/{name}/browser/`:

```
POST   /sandboxes/{name}/browser/start                    # Start browser server
GET    /sandboxes/{name}/browser/pages                     # List pages
POST   /sandboxes/{name}/browser/pages                     # Create page
DELETE /sandboxes/{name}/browser/pages/{page}              # Close page
POST   /sandboxes/{name}/browser/pages/{page}/goto        # Navigate
GET    /sandboxes/{name}/browser/pages/{page}/snapshot     # ARIA snapshot
POST   /sandboxes/{name}/browser/pages/{page}/click       # Click element
POST   /sandboxes/{name}/browser/pages/{page}/fill        # Fill input
POST   /sandboxes/{name}/browser/pages/{page}/screenshot  # PNG screenshot
POST   /sandboxes/{name}/browser/pages/{page}/evaluate    # Run JavaScript
GET    /sandboxes/{name}/browser/pages/{page}/content     # Raw page content
GET    /sandboxes/{name}/browser/events                   # Event stream
```

## Event Stream

Browser events are sequenced for debugging and context recovery:

```json
[
  {"seq": 1, "type": "page.navigated", "page": "default", "ts": "2026-02-10T12:00:00Z"},
  {"seq": 2, "type": "page.clicked",   "page": "default", "ts": "2026-02-10T12:00:01Z"}
]
```

Use `offset` to resume from a known position — useful for agents recovering context after compaction.

## CLI Usage

```bash
agentkernel sandbox create --template playwright my-browser
agentkernel file write my-browser /app/scrape.py < scrape.py
agentkernel exec my-browser -- python3 /app/scrape.py https://example.com
agentkernel sandbox remove my-browser
```

## Configuration

Playwright needs glibc — use `python:3.12-slim` (Debian-based), not Alpine. Chromium uses ~500-800 MB at idle; use at least 2048 MB memory.

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

## See Also

- [Playwright example](https://github.com/thrashr888/agentkernel/tree/main/examples/playwright)
- [Playwright Stealth example](https://github.com/thrashr888/agentkernel/tree/main/examples/playwright-stealth)
- [SDK Reference](../sdks/index.md) — Go, Rust, Swift browser examples
- [MCP Integration](../api/mcp.md)
- [HTTP API](../api/http.md)
