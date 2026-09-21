// SPDX-License-Identifier: AGPL-3.0-or-later
// Captures one element (or the viewport) from a running workbench. Dev-only.
// Usage: node shoot-site.js <url> <out.png> [css-selector]
const { chromium } = require('@playwright/test');
(async () => {
  const [url, out, selector] = process.argv.slice(2);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(1500);
  if (selector) {
    const el = page.locator(selector).first();
    await el.scrollIntoViewIfNeeded();
    await page.waitForTimeout(400);
    await el.screenshot({ path: out });
    console.log('captured element', selector, '->', out);
  } else {
    await page.screenshot({ path: out });
    console.log('captured viewport ->', out);
  }
  await browser.close();
})();
