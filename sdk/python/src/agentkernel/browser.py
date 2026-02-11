"""Browser session for orchestrating headless browsers in sandboxes."""

from __future__ import annotations

import base64
import json
import shlex
from types import TracebackType
from typing import TYPE_CHECKING, Any

from .types import PageLink, PageResult, RunOutput

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
            links=[PageLink(**l) for l in data.get("links", [])],
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
            links=[PageLink(**l) for l in data.get("links", [])],
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
