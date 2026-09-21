// SPDX-License-Identifier: AGPL-3.0-or-later
// Prove a page makes no external request: every request is the document, a sibling
// asset or a data: URI. Scrolls the whole page so lazily-loaded images are included.
// Dev-only; never part of the product.
// Usage: node netcheck.js <file-or-url> [more...]  (exit 1 if anything external or failing)
const { chromium } = require('@playwright/test');
(async () => {
  const targets = process.argv.slice(2);
  const browser = await chromium.launch();
  let bad = 0;
  for (const target of targets) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const requests = [];
    const failed = [];
    page.on('request', (r) => requests.push(r.url()));
    page.on('requestfailed', (r) => failed.push(r.url() + ' :: ' + (r.failure() || {}).errorText));
    page.on('response', (r) => { if (r.status() >= 400) failed.push(r.url() + ' :: HTTP ' + r.status()); });
    await page.goto(target, { waitUntil: 'networkidle', timeout: 60000 });
    await page.evaluate(async () => {
      const step = 500;
      for (let y = 0; y < document.body.scrollHeight; y += step) {
        window.scrollTo(0, y);
        await new Promise((r) => setTimeout(r, 60));
      }
      window.scrollTo(0, 0);
    });
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(400);
    const external = requests.filter((u) => !(u.startsWith('file://') || u.startsWith('http://127.0.0.1') || u.startsWith('http://localhost') || u.startsWith('data:')));
    console.log('=== ' + target);
    console.log('  requests: ' + requests.length + '   problems: ' + failed.length + '   external: ' + external.length);
    for (const u of failed) console.log('    PROBLEM ' + u);
    for (const u of external) console.log('    EXTERNAL ' + u);
    const hosts = new Set(requests.map((u) => { try { return new URL(u).host || '(file)'; } catch { return '(other)'; } }));
    console.log('    hosts touched: ' + Array.from(hosts).join(', '));
    if (external.length || failed.length) bad += external.length + failed.length;
    await page.close();
  }
  await browser.close();
  console.log(bad === 0 ? 'RESULT: no external requests, no failed requests' : 'RESULT: ' + bad + ' problems');
  process.exitCode = bad === 0 ? 0 : 1;
})();