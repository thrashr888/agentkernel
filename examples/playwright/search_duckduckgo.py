"""Search DuckDuckGo and extract result links.

This is a payload script — it runs inside the sandbox.
Orchestrate it from outside via: agentkernel exec my-browser -- python3 search_duckduckgo.py "query"

Note: Search engines block standard headless browsers. For reliable results,
use the playwright-stealth version instead.
"""

import asyncio
import json
import sys
from playwright.async_api import async_playwright


async def main():
    query = sys.argv[1] if len(sys.argv) > 1 else "agentkernel"

    async with async_playwright() as p:
        browser = await p.chromium.launch()
        page = await browser.new_page()

        await page.goto(f"https://duckduckgo.com/?q={query}", wait_until="networkidle")
        await page.wait_for_timeout(3000)

        results = await page.evaluate("""() => {
            const seen = new Set();
            return Array.from(document.querySelectorAll('a[href]'))
                .filter(a => {
                    const h = a.href;
                    if (!h || h.includes('duckduckgo.com') || h.startsWith('javascript:')) return false;
                    if (seen.has(h)) return false;
                    seen.add(h);
                    return a.textContent.trim().length > 2;
                })
                .slice(0, 15)
                .map(a => ({ title: a.textContent.trim().slice(0, 150), url: a.href }));
        }""")

        print(json.dumps(results, indent=2))
        await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
