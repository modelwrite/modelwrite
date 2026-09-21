// SPDX-License-Identifier: AGPL-3.0-or-later
// Capture a clipped region of a running workbench page, with a stylesheet injected.
// Dev-only; never part of the product.
// Usage: node shoot-crop.js <url> <out.png> <theme.css> [selector]  (default selector: main)
// stylesheet injected. Usage: node shoot-crop.js <url> <out.png> <theme.css> [selector]
const fs = require('node:fs');
const { chromium } = require('@playwright/test');
(async () => {
  const [url, out, theme, sel] = process.argv.slice(2);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.addStyleTag({ content: fs.readFileSync(theme, 'utf8') });
  await page.waitForTimeout(500);
  const box = await page.evaluate((s) => {
    const el = document.querySelector(s) || document.querySelector('main');
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  }, sel || 'main');
  const vp = page.viewportSize();
  const clip = {
    x: Math.max(0, Math.round(box.x)),
    y: Math.max(0, Math.round(box.y)),
    width: Math.round(box.w),
    height: Math.round(Math.min(box.h, vp.height - Math.max(0, box.y))),
  };
  await page.screenshot({ path: out, clip });
  console.log('cropped', out, JSON.stringify(clip));
  await browser.close();
})();