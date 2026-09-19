// SPDX-License-Identifier: AGPL-3.0-or-later
const { chromium } = require('@playwright/test');
(async () => {
  const url = process.argv[2];
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(1200);
  // scroll to the Requirements section if present, else mid-page
  const h = await page.evaluate(() => {
    const heads = Array.from(document.querySelectorAll('h2, h3'));
    const t = heads.find((x) => /requirement/i.test(x.textContent));
    if (t) { t.scrollIntoView(); return t.textContent.trim(); }
    window.scrollBy(0, 1200); return 'mid-page';
  });
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'ui-lower.png' });
  console.log('captured lower view at:', h);
  await browser.close();
})();
