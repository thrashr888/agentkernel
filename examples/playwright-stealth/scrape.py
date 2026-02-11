"""Playwright-stealth example: scrape a page while avoiding bot detection."""

import asyncio
from playwright.async_api import async_playwright
from playwright_stealth import Stealth


async def main():
    async with Stealth().use_async(async_playwright()) as p:
        browser = await p.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://example.com")

        title = await page.title()
        print(f"Page title: {title}")

        # Check that stealth evasions are active
        is_automated = await page.evaluate("() => navigator.webdriver")
        print(f"navigator.webdriver: {is_automated}")

        await page.screenshot(path="screenshot.png")
        print("Screenshot saved to screenshot.png")

        await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
