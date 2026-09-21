// SPDX-License-Identifier: AGPL-3.0-or-later
// Capture a running workbench page with one extra stylesheet injected.
// Dev-only; never part of the product.
// Usage: node shoot-themed.js <url> <out.png> <theme.css>
// The injected sheet is how the three visual directions in website/styles.html were
// compared on identical content: one page, three palettes, nothing mocked up.
// Usage: node shoot-themed.js <url> <out.png> <theme.css>
const fs = require('node:fs');
const { chromium } = require('@playwright/test');
(async () => {
  const [url, out, theme] = process.argv.slice(2);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.addStyleTag({ content: fs.readFileSync(theme, 'utf8') });
  await page.waitForTimeout(600);
  await page.screenshot({ path: out });
  console.log('captured', out);
  await browser.close();
})();