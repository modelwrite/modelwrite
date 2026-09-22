// SPDX-License-Identifier: AGPL-3.0-or-later
// Screenshots the STPA completeness page for a model with NO HAZARDS - the model that used to
// render as complete because every relational check was silent over it. Dev-only; never part of
// the product.
//
//   node e2e/shoot-stpa-not-started.mjs [baseURL] [project] [outDir]
import { chromium } from '@playwright/test';

const base = process.argv[2] || 'http://127.0.0.1:8094';
const project = process.argv[3] || 'stpa-no-hazards';
const out = process.argv[4] || 'e2e';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
await page.goto(base + '/ui/projects/' + encodeURIComponent(project) + '/stpa', {
  waitUntil: 'networkidle',
  timeout: 60000,
});
await page.waitForTimeout(500);
await page.screenshot({ path: out + '/stpa-not-started.png', fullPage: true });
const state = await page.locator('[data-mw-stpa-state]').first().getAttribute('data-mw-stpa-state');
console.log('captured', out + '/stpa-not-started.png', '- state:', state);
await browser.close();
