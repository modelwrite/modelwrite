// SPDX-License-Identifier: AGPL-3.0-or-later
// Screenshots the workbench for a visual review. Dev-only; never part of the product.
const { chromium } = require('@playwright/test');

(async () => {
  const url = process.argv[2] || 'http://127.0.0.1:8080/ui/projects/coffee-machine/model?branch=main';
  const out = process.argv[3] || 'ui-shot.png';
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  await page.waitForTimeout(1500);
  await page.screenshot({ path: out, fullPage: false });
  await page.screenshot({ path: out.replace('.png', '-full.png'), fullPage: true });
  console.log('captured', out);
  await browser.close();
})();
