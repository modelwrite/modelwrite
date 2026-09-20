// SPDX-License-Identifier: AGPL-3.0-or-later
const { chromium } = require('@playwright/test');
(async () => {
  const [url, out, term] = process.argv.slice(2);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(1200);
  const found = await page.evaluate((t) => {
    const hs = Array.from(document.querySelectorAll('h2, h3'));
    const hit = hs.find((x) => new RegExp(t, 'i').test(x.textContent));
    if (hit) { hit.scrollIntoView({ block: 'start' }); return hit.textContent.trim(); }
    return 'not found';
  }, term);
  await page.waitForTimeout(500);
  await page.screenshot({ path: out });
  console.log('scrolled to:', found);
  await browser.close();
})();
