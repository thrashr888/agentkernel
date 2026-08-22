"""Browser session for orchestrating headless browsers in sandboxes."""

from __future__ import annotations

import base64
import json
from types import TracebackType
from typing import TYPE_CHECKING, Any, cast

from .types import AriaSnapshot, PageLink, PageResult, RunOutput

if TYPE_CHECKING:
    from .async_client import AsyncAgentKernel
    from .client import AgentKernel

# Inline Playwright script templates.
# Each method generates a self-contained Python script that launches Chromium,
# performs one action, prints JSON to stdout, and exits.

_GOTO_SCRIPT = """
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

_SCREENSHOT_SCRIPT = """
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

_EVALUATE_SCRIPT = """
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

_SETUP_CMD = ["sh", "-c", "pip install -q playwright && playwright install --with-deps chromium"]

# --- v2 scripts (browser server, health check, request proxy, ARIA snapshot) ---

_BROWSER_HEALTH_SCRIPT = """
import json, urllib.request, sys
port = sys.argv[1] if len(sys.argv) > 1 else "9222"
try:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
    print(req.read().decode())
except Exception as e:
    print(json.dumps({"status": "down", "error": str(e)}))
    sys.exit(1)
"""

_BROWSER_REQUEST_SCRIPT = """
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

# The ARIA snapshot JS and browser server script are loaded from files at build time
# or inlined here. For simplicity, they're inline string constants.
# See src/browser_scripts.rs for the canonical versions.

_ARIA_SNAPSHOT_JS = r"""
(() => {
  let refCounter = 0;
  const refMap = {};
  window.__akRefs = refMap;
  const INTERACTIVE_ROLES = new Set([
    'link', 'button', 'textbox', 'checkbox', 'radio', 'combobox',
    'listbox', 'menuitem', 'menuitemcheckbox', 'menuitemradio',
    'option', 'searchbox', 'slider', 'spinbutton', 'switch', 'tab',
    'treeitem', 'select',
  ]);
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
  function getRole(el) { return el.getAttribute('role') || implicitRole(el); }
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
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') {
      if (el.id) { const l = document.querySelector(`label[for="${el.id}"]`); if (l) return l.textContent.trim(); }
      const p = el.closest('label');
      if (p) { const c = p.cloneNode(true); c.querySelectorAll('input,textarea,select').forEach(n => n.remove()); const t = c.textContent.trim(); if (t) return t; }
    }
    if (el.tagName === 'IMG') return el.getAttribute('alt') || '';
    const tag = el.tagName.toLowerCase();
    if (tag === 'button' || tag === 'a' || el.getAttribute('role') === 'button') return el.textContent.trim().slice(0, 200);
    return '';
  }
  function walkNode(el, depth) {
    if (el.nodeType === Node.TEXT_NODE) { const t = el.textContent.trim(); return t ? { text: t.slice(0, 200) } : null; }
    if (el.nodeType !== Node.ELEMENT_NODE) return null;
    const tag = el.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'noscript') return null;
    if (isHidden(el)) return null;
    const role = getRole(el);
    if (role === 'presentation' || role === 'none') return null;
    const node = {};
    if (role) node.role = role;
    const name = accessibleName(el);
    if (name) node.name = name;
    if (role === 'heading') { const m = tag.match(/^h(\d)$/); if (m) node.level = parseInt(m[1]); }
    if (role && INTERACTIVE_ROLES.has(role)) {
      refCounter++;
      const refId = 'e' + refCounter;
      node.ref = refId;
      refMap[refId] = el;
      if (el.checked !== undefined) node.checked = el.checked;
      if (el.disabled) node.disabled = true;
      if (el.getAttribute('aria-expanded') !== null) node.expanded = el.getAttribute('aria-expanded') === 'true';
      if (el.getAttribute('aria-selected') !== null) node.selected = el.getAttribute('aria-selected') === 'true';
      if (role === 'textbox' || role === 'searchbox') { if (el.placeholder) node.placeholder = el.placeholder; if (el.type === 'password') node.type = 'password'; if (el.value) node.value = el.value; }
      if (role === 'link' && el.href) { try { const u = new URL(el.href); node.url = u.pathname + u.search + u.hash || '/'; } catch { node.url = el.getAttribute('href'); } }
    }
    const children = [];
    for (const child of el.childNodes) { const r = walkNode(child, depth + 1); if (r) children.push(r); }
    if (!role && !name && children.length === 1) return children[0];
    if (!role && !name && children.length > 0) return children.length === 1 ? children[0] : { children };
    if (tag === 'p') { const t = el.textContent.trim().slice(0, 200); return t ? { role: 'paragraph', text: t } : null; }
    if (children.length > 0) node.children = children;
    else if (!name && !node.ref) { const t = el.textContent.trim().slice(0, 200); if (t) node.text = t; else return null; }
    return node;
  }
  function toYaml(node, indent) {
    if (!node) return '';
    const pad = '  '.repeat(indent);
    if (node.text && !node.role && !node.children) return pad + '- text: "' + node.text.replace(/"/g, '\\"') + '"\n';
    if (!node.role && node.children) return node.children.map(c => toYaml(c, indent)).join('');
    let line = pad + '- ' + (node.role || 'element');
    if (node.name) line += ' "' + node.name.replace(/"/g, '\\"') + '"';
    if (node.ref) line += ' [ref=' + node.ref + ']';
    if (node.level) line += ' [level=' + node.level + ']';
    if (node.url) line += ' [url=' + node.url + ']';
    if (node.placeholder) line += ' [placeholder=' + node.placeholder + ']';
    if (node.type) line += ' [type=' + node.type + ']';
    if (node.checked !== undefined) line += ' [checked=' + node.checked + ']';
    if (node.disabled) line += ' [disabled]';
    if (node.expanded !== undefined) line += ' [expanded=' + node.expanded + ']';
    if (node.selected !== undefined) line += ' [selected=' + node.selected + ']';
    if (node.value) line += ' [value=' + node.value + ']';
    if (node.text && !node.children) { line += ': "' + node.text.replace(/"/g, '\\"') + '"'; return line + '\n'; }
    if (node.children && node.children.length > 0) { line += ':\n'; return line + node.children.map(c => toYaml(c, indent + 1)).join(''); }
    return line + '\n';
  }
  const tree = walkNode(document.body, 0);
  if (!tree) return '';
  if (tree.children) return tree.children.map(c => toYaml(c, 0)).join('');
  return toYaml(tree, 0);
})()
"""

_BROWSER_SERVER_SCRIPT = r'''
import asyncio, base64, json, sys, os, signal
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs
from threading import Thread
browser = None; context = None; pages = {}; event_log = []; seq_counter = 0
ARIA_SNAPSHOT_JS = None
def next_seq():
    global seq_counter; seq_counter += 1; return seq_counter
def emit_event(t, p, **e):
    from datetime import datetime, timezone
    evt = {"seq": next_seq(), "type": t, "page": p, "ts": datetime.now(timezone.utc).isoformat(), **e}
    event_log.append(evt); return evt
async def ensure_browser():
    global browser, context
    if browser is None:
        from playwright.async_api import async_playwright
        pw = await async_playwright().start(); browser = await pw.chromium.launch(); context = await browser.new_context()
async def get_or_create_page(name):
    await ensure_browser()
    if name not in pages: page = await context.new_page(); pages[name] = page; emit_event("page.created", name)
    return pages[name]
async def take_snapshot(page, pn):
    y = await page.evaluate(ARIA_SNAPSHOT_JS); refs = await page.evaluate("() => window.__akRefs ? Object.keys(window.__akRefs) : []")
    emit_event("page.snapshot", pn, refs=refs); return {"snapshot": y, "url": page.url, "title": await page.title(), "refs": refs}
async def handle_goto(page, pn, body):
    await page.goto(body.get("url",""), timeout=30000); emit_event("page.navigated", pn, url=page.url, title=await page.title()); return await take_snapshot(page, pn)
async def handle_click(page, pn, body):
    ref = body.get("ref"); sel = body.get("selector")
    if ref:
        c = await page.evaluate("""(r) => { const e = window.__akRefs && window.__akRefs[r]; if (!e) return null; e.click(); return {role: e.getAttribute('role') || e.tagName.toLowerCase(), name: e.getAttribute('aria-label') || e.textContent.trim().slice(0, 50)}; }""", ref)
        if c is None: return {"error": f"ref '{ref}' not found. Take a new snapshot to get current refs."}
        emit_event("page.clicked", pn, ref=ref, role=c.get("role"), name=c.get("name"))
    elif sel: await page.click(sel, timeout=5000); emit_event("page.clicked", pn, selector=sel)
    else: return {"error": "ref or selector is required"}
    try: await page.wait_for_load_state("networkidle", timeout=3000)
    except: pass
    return await take_snapshot(page, pn)
async def handle_fill(page, pn, body):
    ref = body.get("ref"); sel = body.get("selector"); val = body.get("value", "")
    if ref:
        f = await page.evaluate("""([r, v]) => { const e = window.__akRefs && window.__akRefs[r]; if (!e) return false; e.focus(); e.value = v; e.dispatchEvent(new Event('input', {bubbles: true})); e.dispatchEvent(new Event('change', {bubbles: true})); return true; }""", [ref, val])
        if not f: return {"error": f"ref '{ref}' not found. Take a new snapshot to get current refs."}
        emit_event("page.filled", pn, ref=ref)
    elif sel: await page.fill(sel, val, timeout=5000); emit_event("page.filled", pn, selector=sel)
    else: return {"error": "ref or selector is required"}
    return await take_snapshot(page, pn)
async def handle_screenshot(page, pn): d = await page.screenshot(); emit_event("page.screenshot", pn); return {"screenshot": base64.b64encode(d).decode()}
async def handle_evaluate(page, pn, body): return {"result": await page.evaluate(body.get("expression", "null"))}
async def handle_content(page, pn):
    t = await page.evaluate("() => document.body.innerText.slice(0, 8000)")
    l = await page.evaluate("() => Array.from(document.querySelectorAll('a[href]')).slice(0,50).map(a=>({text:a.textContent.trim(),href:a.href})).filter(l=>l.href.startsWith('http'))")
    return {"title": await page.title(), "url": page.url, "text": t, "links": l}
loop = None
class Handler(BaseHTTPRequestHandler):
    def log_message(self, f, *a): pass
    def _json(self, s, d): b = json.dumps(d).encode(); self.send_response(s); self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(b))); self.end_headers(); self.wfile.write(b)
    def _body(self): l = int(self.headers.get("Content-Length",0)); return json.loads(self.rfile.read(l)) if l else {}
    def _run(self, c): return asyncio.run_coroutine_threadsafe(c, loop).result(timeout=60)
    def _parse_path(self): p = urlparse(self.path); return p.path.strip("/").split("/"), parse_qs(p.query)
    def do_GET(self):
        path, query = self._parse_path()
        try:
            if path == ["health"]: self._json(200, {"status": "ok", "pages": list(pages.keys())})
            elif path == ["pages"]: self._json(200, {"pages": list(pages.keys())})
            elif len(path)==3 and path[0]=="pages" and path[2]=="snapshot":
                n = path[1]
                if n not in pages: self._json(404, {"error": f"page '{n}' not found"}); return
                self._json(200, self._run(take_snapshot(pages[n], n)))
            elif len(path)==3 and path[0]=="pages" and path[2]=="content":
                n = path[1]
                if n not in pages: self._json(404, {"error": f"page '{n}' not found"}); return
                self._json(200, self._run(handle_content(pages[n], n)))
            elif path == ["events"]:
                o = int(query.get("offset",[0])[0]); l = int(query.get("limit",[100])[0])
                self._json(200, {"events": [e for e in event_log if e["seq"] > o][:l]})
            else: self._json(404, {"error": "not found"})
        except Exception as e: self._json(500, {"error": str(e)})
    def do_POST(self):
        path, _ = self._parse_path()
        try:
            body = self._body()
            if path == ["pages"]: n = body.get("name","default"); self._run(get_or_create_page(n)); self._json(200, {"page": n, "pages": list(pages.keys())})
            elif len(path)==3 and path[0]=="pages":
                n, a = path[1], path[2]
                if n not in pages: self._run(get_or_create_page(n))
                page = pages[n]
                if a=="goto": self._json(200, self._run(handle_goto(page, n, body)))
                elif a=="click": self._json(200, self._run(handle_click(page, n, body)))
                elif a=="fill": self._json(200, self._run(handle_fill(page, n, body)))
                elif a=="screenshot": self._json(200, self._run(handle_screenshot(page, n)))
                elif a=="evaluate": self._json(200, self._run(handle_evaluate(page, n, body)))
                else: self._json(404, {"error": f"unknown action '{a}'"})
            else: self._json(404, {"error": "not found"})
        except Exception as e: self._json(500, {"error": str(e)})
    def do_DELETE(self):
        path, _ = self._parse_path()
        try:
            if len(path)==2 and path[0]=="pages":
                n = path[1]
                if n in pages: self._run(pages[n].close()); del pages[n]; emit_event("page.closed", n); self._json(200, {"closed": n})
                else: self._json(404, {"error": f"page '{n}' not found"})
            else: self._json(404, {"error": "not found"})
        except Exception as e: self._json(500, {"error": str(e)})
def run_async_loop():
    global loop; loop = asyncio.new_event_loop(); asyncio.set_event_loop(loop); loop.run_forever()
if __name__ == "__main__":
    ARIA_SNAPSHOT_JS = sys.argv[1] if len(sys.argv) > 1 else "() => ''"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 9222
    t = Thread(target=run_async_loop, daemon=True); t.start()
    import time
    while loop is None: time.sleep(0.01)
    asyncio.run_coroutine_threadsafe(ensure_browser(), loop).result(timeout=60)
    server = HTTPServer(("127.0.0.1", port), Handler)
    print(json.dumps({"status": "ready", "port": port}), flush=True)
    def shutdown(sig, frame): server.shutdown(); sys.exit(0)
    signal.signal(signal.SIGTERM, shutdown); signal.signal(signal.SIGINT, shutdown)
    server.serve_forever()
'''

_BROWSER_START_SCRIPT = """
import sys, json, subprocess, time, os, signal, select
js_code = sys.argv[1]; port = sys.argv[2] if len(sys.argv) > 2 else "9222"
server_script = sys.argv[3]
with open("/tmp/ak_browser_server.py", "w") as f: f.write(server_script)
proc = subprocess.Popen(["python3", "/tmp/ak_browser_server.py", js_code, port], stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=os.setpgrp)
with open("/tmp/ak_browser_server.pid", "w") as f: f.write(str(proc.pid))
start = time.time()
while time.time() - start < 30:
    ready, _, _ = select.select([proc.stdout], [], [], 1.0)
    if ready:
        line = proc.stdout.readline().decode().strip()
        if line:
            try:
                data = json.loads(line)
                if data.get("status") == "ready": print(json.dumps({"status": "ready", "port": int(port), "pid": proc.pid})); sys.exit(0)
            except json.JSONDecodeError: pass
    if proc.poll() is not None:
        stderr = proc.stderr.read().decode()
        print(json.dumps({"error": f"Server exited with code {proc.returncode}", "stderr": stderr})); sys.exit(1)
print(json.dumps({"error": "Timeout waiting for browser server to start"})); sys.exit(1)
"""


class BrowserSession:
    """A sandboxed headless browser controlled from outside.

    The browser (Chromium via Playwright) runs inside an agentkernel sandbox.
    You call high-level methods; the SDK generates and executes scripts internally.

    Example::

        with client.browser("my-browser") as browser:
            page = browser.goto("https://example.com")
            print(page.title, page.links)
            png = browser.screenshot("https://example.com")
        # sandbox auto-removed
    """

    def __init__(self, name: str, client: AgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False
        self._last_url: str | None = None

    def goto(self, url: str) -> PageResult:
        """Navigate to a URL and return page data (title, text, links)."""
        result = self._run_script(_GOTO_SCRIPT, url)
        data = json.loads(result.output)
        self._last_url = url
        return PageResult(
            title=data["title"],
            url=data["url"],
            text=data["text"],
            links=[PageLink(**link) for link in data.get("links", [])],
        )

    def screenshot(self, url: str | None = None) -> bytes:
        """Take a PNG screenshot. Uses last visited URL if none specified."""
        target = url or self._last_url
        if not target:
            raise ValueError("No URL specified and no previous goto() call")
        result = self._run_script(_SCREENSHOT_SCRIPT, target)
        return base64.b64decode(result.output.strip())

    def evaluate(self, expression: str, url: str | None = None) -> Any:
        """Run a JavaScript expression on a page and return the result."""
        target = url or self._last_url
        if not target:
            raise ValueError("No URL specified and no previous goto() call")
        result = self._run_script(_EVALUATE_SCRIPT, target, expression)
        return json.loads(result.output)

    # --- v2 methods (ARIA snapshots, persistent pages, ref-based interaction) ---

    def open(self, url: str, *, page: str = "default") -> AriaSnapshot:
        """Navigate to a URL and return an ARIA snapshot.

        The browser server is auto-started on first call. Pages persist
        across calls — no cold start per action.
        """
        body = json.dumps({"url": url})
        result = self._browser_request("POST", f"/pages/{page}/goto", body)
        data = json.loads(result.output)
        return AriaSnapshot(**data)

    def snapshot(self, *, page: str = "default") -> AriaSnapshot:
        """Get the current ARIA snapshot without navigating."""
        result = self._browser_request("GET", f"/pages/{page}/snapshot")
        data = json.loads(result.output)
        return AriaSnapshot(**data)

    def click(
        self, *, ref: str | None = None, selector: str | None = None, page: str = "default"
    ) -> AriaSnapshot:
        """Click an element by ref ID or CSS selector. Returns new snapshot."""
        body_dict: dict[str, str] = {}
        if ref:
            body_dict["ref"] = ref
        if selector:
            body_dict["selector"] = selector
        result = self._browser_request("POST", f"/pages/{page}/click", json.dumps(body_dict))
        data = json.loads(result.output)
        if "error" in data:
            raise ValueError(data["error"])
        return AriaSnapshot(**data)

    def fill(
        self,
        value: str,
        *,
        ref: str | None = None,
        selector: str | None = None,
        page: str = "default",
    ) -> AriaSnapshot:
        """Fill an input by ref ID or CSS selector. Returns new snapshot."""
        body_dict: dict[str, str] = {"value": value}
        if ref:
            body_dict["ref"] = ref
        if selector:
            body_dict["selector"] = selector
        result = self._browser_request("POST", f"/pages/{page}/fill", json.dumps(body_dict))
        data = json.loads(result.output)
        if "error" in data:
            raise ValueError(data["error"])
        return AriaSnapshot(**data)

    def close_page(self, page: str = "default") -> None:
        """Close a named page."""
        self._browser_request("DELETE", f"/pages/{page}")

    def list_pages(self) -> list[str]:
        """List active page names."""
        result = self._browser_request("GET", "/pages")
        data = json.loads(result.output)
        return cast(list[str], data.get("pages", []))

    def _browser_request(self, method: str, path: str, body: str | None = None) -> RunOutput:
        """Send a request to the in-sandbox browser server."""
        self._ensure_server()
        cmd = [
            "python3",
            "-c",
            _BROWSER_REQUEST_SCRIPT,
            "9222",
            method,
            path,
        ]
        if body:
            cmd.append(body)
        return self._client.exec_in_sandbox(self.name, cmd)

    def _ensure_server(self) -> None:
        """Ensure the browser server is running."""
        if getattr(self, "_server_started", False):
            return
        # Check health
        try:
            result = self._client.exec_in_sandbox(
                self.name,
                ["python3", "-c", _BROWSER_HEALTH_SCRIPT, "9222"],
            )
            if '"status": "ok"' in result.output or '"status":"ok"' in result.output:
                self._server_started = True
                return
        except Exception:
            pass
        # Start server
        result = self._client.exec_in_sandbox(
            self.name,
            [
                "python3",
                "-c",
                _BROWSER_START_SCRIPT,
                _ARIA_SNAPSHOT_JS,
                "9222",
                _BROWSER_SERVER_SCRIPT,
            ],
        )
        data = json.loads(result.output)
        if "error" in data:
            raise RuntimeError(f"Browser server failed to start: {data['error']}")
        self._server_started = True

    # --- v1 methods (backward compatible) ---

    def remove(self) -> None:
        """Remove the sandbox. Idempotent."""
        if self._removed:
            return
        self._removed = True
        self._client.remove_sandbox(self.name)

    def _run_script(self, script: str, *args: str) -> RunOutput:
        """Execute an inline Python script inside the sandbox."""
        cmd = ["python3", "-c", script, *args]
        return self._client.exec_in_sandbox(self.name, cmd)

    def __enter__(self) -> BrowserSession:
        return self

    def __exit__(self, *args: Any) -> None:
        self.remove()


class AsyncBrowserSession:
    """Async version of BrowserSession."""

    def __init__(self, name: str, client: AsyncAgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False
        self._last_url: str | None = None

    async def goto(self, url: str) -> PageResult:
        """Navigate to a URL and return page data."""
        result = await self._run_script(_GOTO_SCRIPT, url)
        data = json.loads(result.output)
        self._last_url = url
        return PageResult(
            title=data["title"],
            url=data["url"],
            text=data["text"],
            links=[PageLink(**link) for link in data.get("links", [])],
        )

    async def screenshot(self, url: str | None = None) -> bytes:
        """Take a PNG screenshot."""
        target = url or self._last_url
        if not target:
            raise ValueError("No URL specified and no previous goto() call")
        result = await self._run_script(_SCREENSHOT_SCRIPT, target)
        return base64.b64decode(result.output.strip())

    async def evaluate(self, expression: str, url: str | None = None) -> Any:
        """Run a JavaScript expression on a page and return the result."""
        target = url or self._last_url
        if not target:
            raise ValueError("No URL specified and no previous goto() call")
        result = await self._run_script(_EVALUATE_SCRIPT, target, expression)
        return json.loads(result.output)

    # --- v2 methods (ARIA snapshots, persistent pages, ref-based interaction) ---

    async def open(self, url: str, *, page: str = "default") -> AriaSnapshot:
        """Navigate to a URL and return an ARIA snapshot."""
        body = json.dumps({"url": url})
        result = await self._browser_request("POST", f"/pages/{page}/goto", body)
        data = json.loads(result.output)
        return AriaSnapshot(**data)

    async def snapshot(self, *, page: str = "default") -> AriaSnapshot:
        """Get the current ARIA snapshot without navigating."""
        result = await self._browser_request("GET", f"/pages/{page}/snapshot")
        data = json.loads(result.output)
        return AriaSnapshot(**data)

    async def click(
        self, *, ref: str | None = None, selector: str | None = None, page: str = "default"
    ) -> AriaSnapshot:
        """Click an element by ref ID or CSS selector. Returns new snapshot."""
        body_dict: dict[str, str] = {}
        if ref:
            body_dict["ref"] = ref
        if selector:
            body_dict["selector"] = selector
        result = await self._browser_request("POST", f"/pages/{page}/click", json.dumps(body_dict))
        data = json.loads(result.output)
        if "error" in data:
            raise ValueError(data["error"])
        return AriaSnapshot(**data)

    async def fill(
        self,
        value: str,
        *,
        ref: str | None = None,
        selector: str | None = None,
        page: str = "default",
    ) -> AriaSnapshot:
        """Fill an input by ref ID or CSS selector. Returns new snapshot."""
        body_dict: dict[str, str] = {"value": value}
        if ref:
            body_dict["ref"] = ref
        if selector:
            body_dict["selector"] = selector
        result = await self._browser_request("POST", f"/pages/{page}/fill", json.dumps(body_dict))
        data = json.loads(result.output)
        if "error" in data:
            raise ValueError(data["error"])
        return AriaSnapshot(**data)

    async def close_page(self, page: str = "default") -> None:
        """Close a named page."""
        await self._browser_request("DELETE", f"/pages/{page}")

    async def list_pages(self) -> list[str]:
        """List active page names."""
        result = await self._browser_request("GET", "/pages")
        data = json.loads(result.output)
        return cast(list[str], data.get("pages", []))

    async def _browser_request(self, method: str, path: str, body: str | None = None) -> RunOutput:
        """Send a request to the in-sandbox browser server."""
        await self._ensure_server()
        cmd = ["python3", "-c", _BROWSER_REQUEST_SCRIPT, "9222", method, path]
        if body:
            cmd.append(body)
        return await self._client.exec_in_sandbox(self.name, cmd)

    async def _ensure_server(self) -> None:
        """Ensure the browser server is running."""
        if getattr(self, "_server_started", False):
            return
        try:
            result = await self._client.exec_in_sandbox(
                self.name, ["python3", "-c", _BROWSER_HEALTH_SCRIPT, "9222"]
            )
            if '"status": "ok"' in result.output or '"status":"ok"' in result.output:
                self._server_started = True
                return
        except Exception:
            pass
        result = await self._client.exec_in_sandbox(
            self.name,
            [
                "python3",
                "-c",
                _BROWSER_START_SCRIPT,
                _ARIA_SNAPSHOT_JS,
                "9222",
                _BROWSER_SERVER_SCRIPT,
            ],
        )
        data = json.loads(result.output)
        if "error" in data:
            raise RuntimeError(f"Browser server failed to start: {data['error']}")
        self._server_started = True

    # --- v1 methods (backward compatible) ---

    async def remove(self) -> None:
        """Remove the sandbox. Idempotent."""
        if self._removed:
            return
        self._removed = True
        await self._client.remove_sandbox(self.name)

    async def _run_script(self, script: str, *args: str) -> RunOutput:
        """Execute an inline Python script inside the sandbox."""
        cmd = ["python3", "-c", script, *args]
        return await self._client.exec_in_sandbox(self.name, cmd)

    async def __aenter__(self) -> AsyncBrowserSession:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        await self.remove()
