"""Playwright example: scrape a page title and take a screenshot."""

import asyncio
from playwright.async_api import async_playwright


async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://example.com")

        title = await page.title()
        print(f"Page title: {title}")

        await page.screenshot(path="screenshot.png")
        print("Screenshot saved to screenshot.png")

        await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
