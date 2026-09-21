// SPDX-License-Identifier: AGPL-3.0-or-later
// Screenshots the projects page and its card grid for a visual review of the per-card
// model-health summary. Dev-only; never part of the product.
//
//   node e2e/shoot-health-cards.mjs [baseURL] [outDir]
import { chromium } from '@playwright/test';

const base = process.argv[2] || 'http://127.0.0.1:8093';
const out = process.argv[3] || 'e2e';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
await page.goto(base + '/ui', { waitUntil: 'networkidle', timeout: 60000 });
await page.waitForTimeout(800);
await page.screenshot({ path: out + '/health-cards.png', fullPage: true });
await page.locator('ul.projects').screenshot({ path: out + '/health-cards-grid.png' });
console.log('captured', out + '/health-cards.png and ' + out + '/health-cards-grid.png');
await browser.close();
