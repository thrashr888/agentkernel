//! Inline Playwright scripts for browser tools.
//!
//! Shared by MCP tools and HTTP API browser endpoints.
//!
//! ## Architecture (v2)
//!
//! The browser server runs inside the sandbox as a persistent Python HTTP server.
//! It manages Playwright/Chromium and exposes page lifecycle, ARIA snapshots,
//! and ref-based interaction over localhost:9222.
//!
//! v1 scripts (GOTO_SCRIPT, SCREENSHOT_SCRIPT, EVALUATE_SCRIPT) are preserved
//! for backward compatibility.

/// Default Docker image for browser sandboxes.
pub const BROWSER_IMAGE: &str = "python:3.12-slim";

/// Default memory allocation (MB) for browser sandboxes.
pub const BROWSER_MEMORY_MB: u64 = 2048;

/// Port the in-sandbox browser server listens on.
pub const BROWSER_SERVER_PORT: u16 = 9222;

/// Command to install Playwright + Chromium inside the sandbox.
pub const BROWSER_SETUP_CMD: &[&str] = &[
    "sh",
    "-c",
    "pip install -q playwright && playwright install --with-deps chromium",
];

// ---------------------------------------------------------------------------
// v1 scripts (backward compatible, one-shot Chromium per call)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// v2: ARIA snapshot JavaScript (injected into pages via Playwright evaluate)
// ---------------------------------------------------------------------------

/// JavaScript module injected into pages to walk the accessibility tree
/// and produce a structured YAML snapshot with ref IDs on interactive elements.
///
/// Usage from Python: `snapshot_yaml = await page.evaluate(ARIA_SNAPSHOT_JS)`
///
/// Returns a YAML string with semantic roles, names, states, and `[ref=eN]`
/// identifiers on interactive elements.
pub const ARIA_SNAPSHOT_JS: &str = r##"
(() => {
  // Ref counter and map for this snapshot
  let refCounter = 0;
  const refMap = {};
  window.__akRefs = refMap;

  const INTERACTIVE_ROLES = new Set([
    'link', 'button', 'textbox', 'checkbox', 'radio', 'combobox',
    'listbox', 'menuitem', 'menuitemcheckbox', 'menuitemradio',
    'option', 'searchbox', 'slider', 'spinbutton', 'switch', 'tab',
    'treeitem', 'select',
  ]);

  const LANDMARK_ROLES = new Set([
    'banner', 'complementary', 'contentinfo', 'form', 'main',
    'navigation', 'region', 'search',
  ]);

  // Map HTML tags to implicit ARIA roles
  function implicitRole(el) {
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    switch (tag) {
      case 'a': return el.hasAttribute('href') ? 'link' : null;
      case 'button': return 'button';
      case 'input':
        if (type === 'checkbox') return 'checkbox';
        if (type === 'radio') return 'radio';
        if (type === 'range') return 'slider';
        if (type === 'number') return 'spinbutton';
        if (type === 'search') return 'searchbox';
        if (type === 'submit' || type === 'reset' || type === 'button') return 'button';
        return 'textbox';
      case 'select': return 'combobox';
      case 'textarea': return 'textbox';
      case 'nav': return 'navigation';
      case 'main': return 'main';
      case 'header': return 'banner';
      case 'footer': return 'contentinfo';
      case 'aside': return 'complementary';
      case 'form': return 'form';
      case 'section': return el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby') ? 'region' : null;
      case 'h1': case 'h2': case 'h3': case 'h4': case 'h5': case 'h6': return 'heading';
      case 'img': return el.getAttribute('alt') === '' ? 'presentation' : 'img';
      case 'ul': case 'ol': return 'list';
      case 'li': return 'listitem';
      case 'table': return 'table';
      case 'option': return 'option';
      default: return null;
    }
  }

  function getRole(el) {
    return el.getAttribute('role') || implicitRole(el);
  }

  function isHidden(el) {
    if (el.getAttribute('aria-hidden') === 'true') return true;
    const style = window.getComputedStyle(el);
    return style.display === 'none' || style.visibility === 'hidden';
  }

  function accessibleName(el) {
    const label = el.getAttribute('aria-label');
    if (label) return label.trim();
    const labelledBy = el.getAttribute('aria-labelledby');
    if (labelledBy) {
      const names = labelledBy.split(/\s+/).map(id => {
        const ref = document.getElementById(id);
        return ref ? ref.textContent.trim() : '';
      }).filter(Boolean);
      if (names.length) return names.join(' ');
    }
    // For inputs, check associated label
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') {
      if (el.id) {
        const assocLabel = document.querySelector(`label[for="${el.id}"]`);
        if (assocLabel) return assocLabel.textContent.trim();
      }
      const parent = el.closest('label');
      if (parent) {
        // Get label text excluding the input itself
        const clone = parent.cloneNode(true);
        clone.querySelectorAll('input,textarea,select').forEach(n => n.remove());
        const t = clone.textContent.trim();
        if (t) return t;
      }
    }
    // For images
    if (el.tagName === 'IMG') return el.getAttribute('alt') || '';
    // For buttons / links — use textContent
    const tag = el.tagName.toLowerCase();
    if (tag === 'button' || tag === 'a' || el.getAttribute('role') === 'button') {
      return el.textContent.trim().slice(0, 200);
    }
    return '';
  }

  function walkNode(el, depth) {
    if (el.nodeType === Node.TEXT_NODE) {
      const text = el.textContent.trim();
      if (!text) return null;
      return { text: text.slice(0, 200) };
    }
    if (el.nodeType !== Node.ELEMENT_NODE) return null;

    // Skip hidden, script, style, svg internals
    const tag = el.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'noscript') return null;
    if (isHidden(el)) return null;

    const role = getRole(el);

    // Skip decorative images
    if (role === 'presentation' || role === 'none') return null;

    // Build node
    const node = {};
    if (role) node.role = role;

    const name = accessibleName(el);
    if (name) node.name = name;

    // Heading level
    if (role === 'heading') {
      const match = tag.match(/^h(\d)$/);
      if (match) node.level = parseInt(match[1]);
    }

    // Interactive element states + ref
    if (role && INTERACTIVE_ROLES.has(role)) {
      refCounter++;
      const refId = 'e' + refCounter;
      node.ref = refId;
      refMap[refId] = el;

      // States
      if (el.checked !== undefined) node.checked = el.checked;
      if (el.disabled) node.disabled = true;
      if (el.getAttribute('aria-expanded') !== null) {
        node.expanded = el.getAttribute('aria-expanded') === 'true';
      }
      if (el.getAttribute('aria-selected') !== null) {
        node.selected = el.getAttribute('aria-selected') === 'true';
      }
      if (el.getAttribute('aria-pressed') !== null) {
        node.pressed = el.getAttribute('aria-pressed') === 'true';
      }
      // Input specifics
      if (role === 'textbox' || role === 'searchbox') {
        if (el.placeholder) node.placeholder = el.placeholder;
        if (el.type === 'password') node.type = 'password';
        if (el.value) node.value = el.value;
      }
      // Link URL
      if (role === 'link' && el.href) {
        try {
          const url = new URL(el.href);
          node.url = url.pathname + url.search + url.hash || '/';
        } catch { node.url = el.getAttribute('href'); }
      }
    }

    // Walk children
    const children = [];
    for (const child of el.childNodes) {
      const result = walkNode(child, depth + 1);
      if (result) children.push(result);
    }

    // Flatten: if generic container (no role, no name) with single child, return child
    if (!role && !name && children.length === 1) {
      return children[0];
    }

    // If this node has no role and no name, just return children as a group
    if (!role && !name && children.length > 0) {
      // Return children directly if it's just text accumulation
      if (children.length === 1) return children[0];
      return { children };
    }

    // Paragraph — just return text content
    if (tag === 'p') {
      const text = el.textContent.trim().slice(0, 200);
      if (!text) return null;
      return { role: 'paragraph', text };
    }

    if (children.length > 0) {
      node.children = children;
    } else if (!name && !node.ref) {
      // Leaf node with no content — skip
      const text = el.textContent.trim().slice(0, 200);
      if (text) node.text = text;
      else return null;
    }

    return node;
  }

  // Format as YAML
  function toYaml(node, indent) {
    if (!node) return '';
    const pad = '  '.repeat(indent);

    // Plain text node
    if (node.text && !node.role && !node.children) {
      return pad + '- text: "' + node.text.replace(/"/g, '\\"') + '"\n';
    }

    // Group without role (anonymous container)
    if (!node.role && node.children) {
      return node.children.map(c => toYaml(c, indent)).join('');
    }

    let line = pad + '- ' + (node.role || 'element');

    // Name
    if (node.name) line += ' "' + node.name.replace(/"/g, '\\"') + '"';

    // Attributes
    if (node.ref) line += ' [ref=' + node.ref + ']';
    if (node.level) line += ' [level=' + node.level + ']';
    if (node.url) line += ' [url=' + node.url + ']';
    if (node.placeholder) line += ' [placeholder=' + node.placeholder + ']';
    if (node.type) line += ' [type=' + node.type + ']';
    if (node.checked !== undefined) line += ' [checked=' + node.checked + ']';
    if (node.disabled) line += ' [disabled]';
    if (node.expanded !== undefined) line += ' [expanded=' + node.expanded + ']';
    if (node.selected !== undefined) line += ' [selected=' + node.selected + ']';
    if (node.pressed !== undefined) line += ' [pressed=' + node.pressed + ']';
    if (node.value) line += ' [value=' + node.value + ']';

    // Leaf with text
    if (node.text && !node.children) {
      line += ': "' + node.text.replace(/"/g, '\\"') + '"';
      return line + '\n';
    }

    // Children
    if (node.children && node.children.length > 0) {
      line += ':\n';
      return line + node.children.map(c => toYaml(c, indent + 1)).join('');
    }

    return line + '\n';
  }

  const tree = walkNode(document.body, 0);
  if (!tree) return '';
  if (tree.children) {
    return tree.children.map(c => toYaml(c, 0)).join('');
  }
  return toYaml(tree, 0);
})()
"##;

// ---------------------------------------------------------------------------
// v2: In-sandbox browser server (Python, persistent Playwright + Chromium)
// ---------------------------------------------------------------------------

/// Python HTTP server that runs inside the sandbox, managing Playwright pages.
///
/// Written to `/tmp/ak_browser_server.py` and started as a background process.
/// Listens on localhost:9222.
///
/// Endpoints:
///   POST   /pages               — create or get named page
///   GET    /pages               — list active pages
///   DELETE /pages/<name>         — close page
///   POST   /pages/<name>/goto   — navigate to URL
///   GET    /pages/<name>/snapshot — ARIA snapshot
///   GET    /pages/<name>/content — raw text content (v1 compat)
///   POST   /pages/<name>/click  — click by ref or selector
///   POST   /pages/<name>/fill   — fill input by ref or selector
///   POST   /pages/<name>/screenshot — PNG screenshot (base64)
///   POST   /pages/<name>/evaluate — run JS expression
///   GET    /health              — liveness check
pub const BROWSER_SERVER_SCRIPT: &str = r##"
import asyncio, base64, json, sys, os, signal
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs
from threading import Thread

# --- Global state ---
browser = None
context = None
pages = {}  # name -> Page
event_log = []  # sequenced events
seq_counter = 0

ARIA_SNAPSHOT_JS = None  # Injected at startup via argv[1]

def next_seq():
    global seq_counter
    seq_counter += 1
    return seq_counter

def emit_event(event_type, page_name, **extra):
    from datetime import datetime, timezone
    evt = {"seq": next_seq(), "type": event_type, "page": page_name,
           "ts": datetime.now(timezone.utc).isoformat(), **extra}
    event_log.append(evt)
    return evt

async def ensure_browser():
    global browser, context
    if browser is None:
        from playwright.async_api import async_playwright
        pw = await async_playwright().start()
        browser = await pw.chromium.launch()
        context = await browser.new_context()

async def get_or_create_page(name):
    await ensure_browser()
    if name not in pages:
        page = await context.new_page()
        pages[name] = page
        emit_event("page.created", name)
    return pages[name]

async def take_snapshot(page, page_name):
    yaml_str = await page.evaluate(ARIA_SNAPSHOT_JS)
    refs = await page.evaluate("() => window.__akRefs ? Object.keys(window.__akRefs) : []")
    emit_event("page.snapshot", page_name, refs=refs)
    return {"snapshot": yaml_str, "url": page.url, "title": await page.title(), "refs": refs}

async def handle_goto(page, page_name, body):
    url = body.get("url", "")
    await page.goto(url, timeout=30000)
    emit_event("page.navigated", page_name, url=page.url, title=await page.title())
    snap = await take_snapshot(page, page_name)
    return snap

async def handle_click(page, page_name, body):
    ref = body.get("ref")
    selector = body.get("selector")
    if ref:
        # Click by ref using stored element reference
        clicked = await page.evaluate("""(refId) => {
            const el = window.__akRefs && window.__akRefs[refId];
            if (!el) return null;
            el.click();
            return {role: el.getAttribute('role') || el.tagName.toLowerCase(),
                    name: el.getAttribute('aria-label') || el.textContent.trim().slice(0, 50)};
        }""", ref)
        if clicked is None:
            return {"error": f"ref '{ref}' not found. Take a new snapshot to get current refs."}
        emit_event("page.clicked", page_name, ref=ref, role=clicked.get("role"), name=clicked.get("name"))
    elif selector:
        await page.click(selector, timeout=5000)
        emit_event("page.clicked", page_name, selector=selector)
    else:
        return {"error": "ref or selector is required"}
    # Wait for potential navigation/network
    try:
        await page.wait_for_load_state("networkidle", timeout=3000)
    except Exception:
        pass
    return await take_snapshot(page, page_name)

async def handle_fill(page, page_name, body):
    ref = body.get("ref")
    selector = body.get("selector")
    value = body.get("value", "")
    if ref:
        filled = await page.evaluate("""([refId, val]) => {
            const el = window.__akRefs && window.__akRefs[refId];
            if (!el) return false;
            el.focus();
            el.value = val;
            el.dispatchEvent(new Event('input', {bubbles: true}));
            el.dispatchEvent(new Event('change', {bubbles: true}));
            return true;
        }""", [ref, value])
        if not filled:
            return {"error": f"ref '{ref}' not found. Take a new snapshot to get current refs."}
        emit_event("page.filled", page_name, ref=ref)
    elif selector:
        await page.fill(selector, value, timeout=5000)
        emit_event("page.filled", page_name, selector=selector)
    else:
        return {"error": "ref or selector is required"}
    return await take_snapshot(page, page_name)

async def handle_screenshot(page, page_name):
    data = await page.screenshot()
    emit_event("page.screenshot", page_name)
    return {"screenshot": base64.b64encode(data).decode()}

async def handle_evaluate(page, page_name, body):
    expr = body.get("expression", "null")
    result = await page.evaluate(expr)
    return {"result": result}

async def handle_content(page, page_name):
    text = await page.evaluate("() => document.body.innerText.slice(0, 8000)")
    links = await page.evaluate('''() =>
        Array.from(document.querySelectorAll('a[href]'))
            .slice(0, 50)
            .map(a => ({text: a.textContent.trim(), href: a.href}))
            .filter(l => l.href.startsWith("http"))
    ''')
    title = await page.title()
    return {"title": title, "url": page.url, "text": text, "links": links}

# --- HTTP Handler (sync wrapper around async) ---

loop = None

class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # Suppress request logging

    def _json(self, status, data):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _body(self):
        length = int(self.headers.get("Content-Length", 0))
        if length == 0:
            return {}
        return json.loads(self.rfile.read(length))

    def _run(self, coro):
        return asyncio.run_coroutine_threadsafe(coro, loop).result(timeout=60)

    def _parse_path(self):
        parts = urlparse(self.path)
        path = parts.path.strip("/").split("/")
        query = parse_qs(parts.query)
        return path, query

    def do_GET(self):
        path, query = self._parse_path()
        try:
            if path == ["health"]:
                self._json(200, {"status": "ok", "pages": list(pages.keys())})
            elif path == ["pages"]:
                self._json(200, {"pages": list(pages.keys())})
            elif len(path) == 3 and path[0] == "pages" and path[2] == "snapshot":
                name = path[1]
                if name not in pages:
                    self._json(404, {"error": f"page '{name}' not found"})
                    return
                snap = self._run(take_snapshot(pages[name], name))
                self._json(200, snap)
            elif len(path) == 3 and path[0] == "pages" and path[2] == "content":
                name = path[1]
                if name not in pages:
                    self._json(404, {"error": f"page '{name}' not found"})
                    return
                result = self._run(handle_content(pages[name], name))
                self._json(200, result)
            elif path == ["events"]:
                offset = int(query.get("offset", [0])[0])
                limit = int(query.get("limit", [100])[0])
                events = [e for e in event_log if e["seq"] > offset][:limit]
                self._json(200, {"events": events})
            else:
                self._json(404, {"error": "not found"})
        except Exception as e:
            self._json(500, {"error": str(e)})

    def do_POST(self):
        path, _ = self._parse_path()
        try:
            body = self._body()
            if path == ["pages"]:
                name = body.get("name", "default")
                self._run(get_or_create_page(name))
                self._json(200, {"page": name, "pages": list(pages.keys())})
            elif len(path) == 3 and path[0] == "pages":
                name = path[1]
                action = path[2]
                if name not in pages:
                    # Auto-create page
                    self._run(get_or_create_page(name))
                page = pages[name]
                if action == "goto":
                    self._json(200, self._run(handle_goto(page, name, body)))
                elif action == "click":
                    self._json(200, self._run(handle_click(page, name, body)))
                elif action == "fill":
                    self._json(200, self._run(handle_fill(page, name, body)))
                elif action == "screenshot":
                    self._json(200, self._run(handle_screenshot(page, name)))
                elif action == "evaluate":
                    self._json(200, self._run(handle_evaluate(page, name, body)))
                else:
                    self._json(404, {"error": f"unknown action '{action}'"})
            else:
                self._json(404, {"error": "not found"})
        except Exception as e:
            self._json(500, {"error": str(e)})

    def do_DELETE(self):
        path, _ = self._parse_path()
        try:
            if len(path) == 2 and path[0] == "pages":
                name = path[1]
                if name in pages:
                    self._run(pages[name].close())
                    del pages[name]
                    emit_event("page.closed", name)
                    self._json(200, {"closed": name})
                else:
                    self._json(404, {"error": f"page '{name}' not found"})
            else:
                self._json(404, {"error": "not found"})
        except Exception as e:
            self._json(500, {"error": str(e)})

def run_async_loop():
    global loop
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    loop.run_forever()

if __name__ == "__main__":
    ARIA_SNAPSHOT_JS = sys.argv[1] if len(sys.argv) > 1 else "() => ''"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 9222

    # Start async loop in background thread
    t = Thread(target=run_async_loop, daemon=True)
    t.start()

    # Wait for loop to be ready
    import time
    while loop is None:
        time.sleep(0.01)

    # Pre-launch browser
    asyncio.run_coroutine_threadsafe(ensure_browser(), loop).result(timeout=60)

    server = HTTPServer(("127.0.0.1", port), Handler)
    print(json.dumps({"status": "ready", "port": port}), flush=True)

    def shutdown(sig, frame):
        server.shutdown()
        if browser:
            asyncio.run_coroutine_threadsafe(browser.close(), loop).result(timeout=10)
        sys.exit(0)

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    server.serve_forever()
"##;

/// Script to start the browser server as a background process.
/// Writes ARIA_SNAPSHOT_JS to a file, starts the server, waits for ready.
pub const BROWSER_SERVER_START_CMD: &str = r#"
import sys, json, subprocess, time, os, signal

js_code = sys.argv[1]
port = sys.argv[2] if len(sys.argv) > 2 else "9222"

# Write the server script
server_script = sys.argv[3]
with open("/tmp/ak_browser_server.py", "w") as f:
    f.write(server_script)

# Start server as background process
proc = subprocess.Popen(
    ["python3", "/tmp/ak_browser_server.py", js_code, port],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    preexec_fn=os.setpgrp
)

# Write PID so we can kill it later
with open("/tmp/ak_browser_server.pid", "w") as f:
    f.write(str(proc.pid))

# Wait for ready signal (up to 30s)
import select
start = time.time()
while time.time() - start < 30:
    ready, _, _ = select.select([proc.stdout], [], [], 1.0)
    if ready:
        line = proc.stdout.readline().decode().strip()
        if line:
            try:
                data = json.loads(line)
                if data.get("status") == "ready":
                    print(json.dumps({"status": "ready", "port": int(port), "pid": proc.pid}))
                    sys.exit(0)
            except json.JSONDecodeError:
                pass
    # Check if process died
    if proc.poll() is not None:
        stderr = proc.stderr.read().decode()
        print(json.dumps({"error": f"Server exited with code {proc.returncode}", "stderr": stderr}))
        sys.exit(1)

print(json.dumps({"error": "Timeout waiting for browser server to start"}))
sys.exit(1)
"#;

/// Script to check if the browser server is running.
pub const BROWSER_SERVER_HEALTH_CMD: &str = r#"
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
"#;

/// Send a request to the in-sandbox browser server.
/// Args: sys.argv[1] = method, sys.argv[2] = path, sys.argv[3] = body JSON (optional)
pub const BROWSER_SERVER_REQUEST_CMD: &str = r#"
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
