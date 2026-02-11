# RFC: Browser Automation v2 — ARIA Snapshots & Persistent Pages

## Problem

Our current browser automation (`BrowserSession` across all 5 SDKs + 5 MCP tools) works but produces poor content for agent consumption:

1. **`goto()` returns 8KB of `innerText`** — unstructured, no semantic meaning, no indication of what's interactive. An agent reading "Home Docs Pricing Login Welcome to Example..." can't tell what's a link vs a heading vs a button.

2. **No persistent state** — every SDK method launches a fresh Chromium, navigates, does one thing, and exits. A 3-step flow (navigate → fill form → submit) requires 3 cold browser launches, each re-navigating from scratch.

3. **CSS selectors for interaction** — agents must guess selectors (`button.submit`, `input[name=email]`) that break on minor DOM changes. No stable element references.

4. **No inspection without navigation** — you can't ask "what's on the page now?" without re-navigating. No way to check state after a click.

These problems compound: an agent using our browser tools spends most of its tokens on selector guessing and re-navigation, not on the actual task.

## Prior Art

### dev-browser (SawyerHood/dev-browser)

Claude Code plugin with the right content model:

- **ARIA snapshots**: Walks the accessibility tree, produces YAML with semantic roles, names, and stable `[ref=e1]` identifiers for interactive elements
- **Persistent pages**: Stateful server keeps pages alive across script executions
- **Ref-based targeting**: `selectSnapshotRef("e5")` returns an ElementHandle — no CSS selectors needed
- **Two modes**: Full script execution or step-by-step exploration

Architecture: Node.js server wrapping Playwright, exposed via HTTP + CDP WebSocket. The ARIA snapshot walker handles shadow DOM, computed visibility, ARIA roles/states, and produces compact YAML (~1-2KB vs 8KB text).

### sandbox-agent (rivet-dev/sandbox-agent)

Unified HTTP API for controlling coding agents in sandboxes:

- **Named sessions as resources**: `POST /v1/sessions/{id}` with lifecycle management
- **SSE event streaming**: Real-time agent events over Server-Sent Events
- **Clean REST model**: Sessions are sub-resources with standard CRUD

Relevant pattern: treating stateful processes as named REST sub-resources with event streams.

## Design Goals

1. **Agent-optimal content** — return ARIA snapshots by default, not raw text. An agent should be able to read a page, identify interactive elements, and act on them in 1-2 tool calls instead of 5-10.

2. **Persistent pages** — navigate once, interact many times. Kill the cold-start-per-action overhead.

3. **Ref-based interaction** — `click(ref="e5")` instead of `click("button.submit")`. Refs are stable within a snapshot and survive across interactions until the page navigates.

4. **Backward compatible** — existing `goto()`, `screenshot()`, `evaluate()` SDK methods keep working. New capabilities are additive.

5. **Sandbox-native** — the browser server runs inside the sandbox like everything else. No host-side browser processes. Full isolation.

## Architecture

```
┌─────────────────────────────────────────────────┐
│  Host                                            │
│                                                  │
│  Agent ──→ MCP/SDK ──→ agentkernel HTTP API     │
│                          │                       │
│                    /sandboxes/:name/browser/*     │
│                          │                       │
│                     exec / port-forward           │
│                          │                       │
│  ┌───────────────────────▼──────────────────┐   │
│  │  Sandbox (container)                      │   │
│  │                                           │   │
│  │  browser-server (Python)                  │   │
│  │    ├── Playwright + Chromium              │   │
│  │    ├── Page registry (named pages)        │   │
│  │    ├── ARIA snapshot engine               │   │
│  │    └── Ref tracker (per-page)             │   │
│  │                                           │   │
│  │  Listens on localhost:9222                │   │
│  └───────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘
```

### Component 1: In-Sandbox Browser Server

A Python HTTP server (~300 lines) that runs inside the sandbox, managing Playwright and page state.

**Why Python**: We already install Playwright via pip. No additional runtime dependency. Keeps the server co-located with the browser in the same language the scripts already use.

**Endpoints**:

```
POST   /pages                      → create or get named page
GET    /pages                      → list active pages
DELETE /pages/:name                → close page

POST   /pages/:name/goto          → navigate to URL
GET    /pages/:name/snapshot       → ARIA snapshot (structured YAML)
GET    /pages/:name/content        → raw text content (backward compat)
POST   /pages/:name/click         → click element by ref or selector
POST   /pages/:name/fill          → fill input by ref or selector
POST   /pages/:name/select        → select option by ref
POST   /pages/:name/screenshot    → PNG screenshot (base64)
POST   /pages/:name/evaluate      → run JS expression
GET    /pages/:name/url            → current URL
POST   /pages/:name/wait          → wait for selector/navigation/idle
```

**Lifecycle**:
- Auto-started on first browser operation (lazy init)
- Auto-installs Playwright + Chromium if missing (one-time, cached in sandbox)
- Pages persist until explicitly closed or sandbox stops
- Server exits when sandbox stops (normal container lifecycle)

### Component 2: ARIA Snapshot Engine

The core value. A JavaScript module injected into pages that walks the accessibility tree and produces structured output.

**Output format** (YAML, compact):

```yaml
- navigation "Main Nav":
  - link "Home" [ref=e1] [url=/]
  - link "Products" [ref=e2] [url=/products]
  - link "Login" [ref=e3] [url=/login]
- main:
  - heading "Welcome to Acme" [level=1]
  - paragraph: "Build better software with Acme tools."
  - form "Sign Up":
    - textbox "Email" [ref=e4] [placeholder=you@example.com]
    - textbox "Password" [ref=e5] [type=password]
    - checkbox "Remember me" [ref=e6] [checked=false]
    - button "Create Account" [ref=e7]
  - region "Features":
    - heading "Fast" [level=2]
    - paragraph: "Sub-second builds."
    - heading "Secure" [level=2]
    - paragraph: "Zero-trust by default."
```

**What gets included**:
- Semantic roles from ARIA + implicit HTML5 roles
- Accessible names (aria-label → aria-labelledby → content)
- Interactive element states (checked, disabled, expanded, selected, pressed)
- Ref IDs on all interactive elements (links, buttons, inputs, selects)
- Link URLs and input types/placeholders
- Heading levels
- Text content (truncated per-node to ~200 chars)

**What gets excluded**:
- Hidden elements (aria-hidden, display:none, visibility:hidden)
- Decorative images (role=presentation/none)
- Style/script elements
- SVG internals (just the top-level role)
- Redundant container nesting (generic divs with single children get flattened)

**Size budget**: Target <4KB for typical pages. The 50-link, 8KB-text dump we return today gets replaced by ~1-2KB of structured YAML that's more actionable.

**Ref lifecycle**:
- Refs are assigned per-snapshot (e1, e2, e3...)
- Stored in `window.__akRefs` Map inside the page
- Stable until next snapshot or navigation
- `click(ref="e5")` looks up the element from the Map

### Component 3: Host API Routes

New routes under the existing sandbox namespace:

```
# Browser server lifecycle
POST   /sandboxes/:name/browser/start     → ensure browser server is running
DELETE /sandboxes/:name/browser/stop       → stop browser server

# Page management
POST   /sandboxes/:name/browser/pages      → create/get page
GET    /sandboxes/:name/browser/pages      → list pages
DELETE /sandboxes/:name/browser/pages/:pg  → close page

# Page interaction
POST   /sandboxes/:name/browser/pages/:pg/goto
GET    /sandboxes/:name/browser/pages/:pg/snapshot
POST   /sandboxes/:name/browser/pages/:pg/click
POST   /sandboxes/:name/browser/pages/:pg/fill
POST   /sandboxes/:name/browser/pages/:pg/screenshot
POST   /sandboxes/:name/browser/pages/:pg/evaluate
POST   /sandboxes/:name/browser/pages/:pg/wait
```

Implementation: thin proxy layer. Each route execs into the sandbox to hit the browser server's localhost endpoint. If the sandbox has port forwarding, use that; otherwise use `exec curl` as the transport.

### Component 4: MCP Tools (v2)

Replace the current 5 tools with a more capable set:

| Tool | Parameters | Returns |
|------|-----------|---------|
| `browser_open` | sandbox, url, page_name? | ARIA snapshot + screenshot |
| `browser_snapshot` | sandbox, page_name? | ARIA snapshot (current state) |
| `browser_click` | sandbox, ref OR selector, page_name? | ARIA snapshot (after click) |
| `browser_fill` | sandbox, ref OR selector, value, page_name? | ARIA snapshot (after fill) |
| `browser_screenshot` | sandbox, page_name? | PNG image |
| `browser_evaluate` | sandbox, expression, page_name? | JS result |
| `browser_close` | sandbox, page_name? | confirmation |

**Key design decisions**:

- **`browser_open` returns snapshot + screenshot together** — one tool call to navigate AND understand the page. Today this takes 2 calls (goto + screenshot or evaluate).
- **`browser_click` and `browser_fill` return the new snapshot** — agent sees the result of its action immediately, no extra call needed.
- **Default page name** — if `page_name` is omitted, use "default". Most workflows use a single page.
- **Ref OR selector** — refs are preferred, but selectors still work as fallback. Agent can use `ref=e5` from the snapshot or fall back to CSS.
- **Auto-start** — if browser server isn't running, start it on first tool call. No explicit setup step.

### Component 5: SDK Changes

New methods on `BrowserSession` (all SDKs):

```python
# Python example

# Existing (backward compatible, now uses persistent page internally)
page = browser.goto("https://example.com")      # returns PageResult (text, title, links)
png  = browser.screenshot()
val  = browser.evaluate("document.title")

# New
snap = browser.snapshot()                         # returns AriaSnapshot (YAML + refs)
snap = browser.open("https://example.com")        # goto + snapshot in one call
snap = browser.click(ref="e5")                    # click by ref, returns new snapshot
snap = browser.click(selector="button.submit")    # click by selector (fallback)
snap = browser.fill(ref="e3", value="user@ex.com") # fill input, returns new snapshot
browser.close_page("checkout")                    # close specific page
pages = browser.list_pages()                      # list active pages
```

**New return type — `AriaSnapshot`**:

```python
@dataclass
class AriaSnapshot:
    yaml: str              # The ARIA tree as YAML
    url: str               # Current page URL
    title: str             # Page title
    refs: list[str]        # Available ref IDs (e.g., ["e1", "e2", ...])
```

## `goto()` Response: Before vs After

**Before** (current):
```json
{
  "title": "Acme - Build Better Software",
  "url": "https://acme.com",
  "text": "Home Products Pricing Login Welcome to Acme Build better software with Acme tools. Email Password Remember me Create Account Fast Sub-second builds. Secure Zero-trust by default. ...",
  "links": [
    {"text": "Home", "href": "https://acme.com/"},
    {"text": "Products", "href": "https://acme.com/products"},
    ...50 more...
  ]
}
```

Agent sees a wall of text. Can't distinguish nav from content from form. Links are a flat list with no context about where they appear.

**After** (v2, via `open()` or `snapshot()`):
```yaml
- navigation "Main Nav":
  - link "Home" [ref=e1] [url=/]
  - link "Products" [ref=e2] [url=/products]
  - link "Login" [ref=e3] [url=/login]
- main:
  - heading "Welcome to Acme" [level=1]
  - paragraph: "Build better software with Acme tools."
  - form "Sign Up":
    - textbox "Email" [ref=e4] [placeholder=you@example.com]
    - textbox "Password" [ref=e5] [type=password]
    - checkbox "Remember me" [ref=e6] [checked=false]
    - button "Create Account" [ref=e7]
  - region "Features":
    - heading "Fast" [level=2]
    - paragraph: "Sub-second builds."
```

Agent immediately sees: 3 nav links, a sign-up form with 3 fields and a submit button, and a features section. It can fill the form in 3 tool calls: `fill(e4, "user@acme.com")`, `fill(e5, "pass123")`, `click(e7)`.

## Implementation Phases

### Phase 1: ARIA Snapshot Engine

The highest-value change with the smallest blast radius.

- [ ] Port dev-browser's ARIA snapshot walker to a standalone JS module (`src/browser_scripts/aria_snapshot.js`)
- [ ] Handle: roles, names, states, refs, visibility, heading levels, link URLs, input types
- [ ] Flatten redundant generic containers
- [ ] Cap per-node text at 200 chars, total output at 4KB
- [ ] Add `browser_snapshot` MCP tool (works with existing `browser_create` sandbox)
- [ ] Update `browser_goto` to return ARIA snapshot alongside existing text response
- [ ] Tests: snapshot a known HTML fixture, verify YAML structure and ref assignment

### Phase 2: In-Sandbox Browser Server

Persistent pages eliminate the cold-start overhead.

- [ ] Write Python browser server (~300 lines, stdlib `http.server` + Playwright)
- [ ] Page registry: create, list, close named pages
- [ ] Auto-install Playwright on first request if missing
- [ ] Auto-start server on first browser MCP tool call
- [ ] Inject ARIA snapshot script into pages on creation
- [ ] Ref tracking: `window.__akRefs` Map per page, lookup on click/fill
- [ ] Health check endpoint for liveness probing

### Phase 3: Ref-Based Interaction

The payoff of ARIA snapshots: agents can act on what they see.

- [ ] `browser_click` MCP tool: accept `ref` or `selector`, return new snapshot
- [ ] `browser_fill` MCP tool: accept `ref` or `selector` + value, return new snapshot
- [ ] Host API proxy routes: `/sandboxes/:name/browser/pages/:pg/*`
- [ ] Error messages include available refs when a ref is not found
- [ ] SDK methods: `click(ref=)`, `fill(ref=, value=)`, `snapshot()`, `open(url)`

### Phase 4: SDK & Backward Compatibility

- [ ] Add `AriaSnapshot` return type to all 5 SDKs
- [ ] Add `open()`, `snapshot()`, `click()`, `fill()`, `list_pages()`, `close_page()` to all 5 SDKs
- [ ] Existing `goto()` keeps returning `PageResult` (text + links) — internally backed by persistent page now
- [ ] Update `browser_create` MCP tool to auto-start browser server instead of manual Playwright install
- [ ] Deprecate but don't remove old `browser_goto` text-only response
- [ ] Update docs: browser-automation.md, SDK pages

## Browser Event Stream

Inspired by [sandbox-agent's session management](https://sandboxagent.dev/docs/manage-sessions), the in-sandbox browser server emits a sequenced event stream. This enables agent context recovery after disconnects and provides an audit trail of browser interactions.

**Event model**:

```json
{"seq": 1, "type": "page.created",    "page": "default", "ts": "..."}
{"seq": 2, "type": "page.navigated",  "page": "default", "url": "https://acme.com", "title": "Acme", "ts": "..."}
{"seq": 3, "type": "page.snapshot",   "page": "default", "refs": ["e1","e2","e3","e4","e5","e6","e7"], "ts": "..."}
{"seq": 4, "type": "page.clicked",    "page": "default", "ref": "e3", "role": "link", "name": "Login", "ts": "..."}
{"seq": 5, "type": "page.navigated",  "page": "default", "url": "https://acme.com/login", "title": "Login", "ts": "..."}
{"seq": 6, "type": "page.filled",     "page": "default", "ref": "e2", "role": "textbox", "name": "Email", "ts": "..."}
{"seq": 7, "type": "page.screenshot", "page": "default", "ts": "..."}
{"seq": 8, "type": "page.closed",     "page": "default", "ts": "..."}
```

**Key design decisions**:

- **Sequence numbers** — monotonic per-sandbox, enabling offset-based resumption. An agent that disconnects can `GET /sandboxes/:name/browser/events?offset=5` to catch up.
- **Memory-only** — events live in the browser server process. When the sandbox stops, they're gone. This is fine — the sandbox is the ephemeral unit, not the event log. If callers need durability, they stream events out (like sandbox-agent's model).
- **Lightweight** — events are metadata only (no snapshot YAML in the event, just ref counts). The full snapshot is returned in the API response; the event stream is for replay/audit.
- **SSE endpoint** — `GET /sandboxes/:name/browser/events/stream` for real-time SSE, same pattern as our existing `/run/stream`. Useful for the desktop app to show live browser activity.

**Host API routes**:

```
GET  /sandboxes/:name/browser/events           → list events (offset, limit)
GET  /sandboxes/:name/browser/events/stream    → SSE stream (offset)
```

**MCP tool**: `browser_events(sandbox, offset?, limit?)` — returns recent events as JSON array. Useful for agents to reconstruct "what happened" without re-snapshotting.

**Desktop app integration**: The events feed into the existing activity toast system. When a browser action happens, the desktop app shows "Navigated to acme.com/login" or "Clicked Login button" in real-time.

## Open Questions

1. **Snapshot script language**: Port the dev-browser TypeScript walker to plain JS (for browser injection), or write a simpler version from scratch using `aria-*` attributes + role mapping?

2. **Transport to in-sandbox server**: Port-forward (cleaner, requires port mapping) vs `exec curl` (works everywhere, slower)? Could support both with port-forward preferred.

3. **Multi-page default**: Should `browser_open` auto-close the previous page (single-page mode) unless `page_name` is explicitly given? Simpler mental model for agents.

4. **Snapshot size for complex pages**: Gmail, GitHub, etc. could produce >4KB snapshots even with filtering. Should we support `depth` or `selector` parameters to scope the snapshot to a region?

5. **Shadow DOM**: Do we need cross-shadow-DOM snapshot support from day 1, or can we add it later? Most agent-relevant pages (docs, dashboards, simple apps) don't use shadow DOM heavily.
