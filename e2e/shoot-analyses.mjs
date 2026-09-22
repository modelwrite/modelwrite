// SPDX-License-Identifier: AGPL-3.0-or-later
// Screenshots the analyses library, one analysis run, and the findings diff between two runs.
// Dev-only; never part of the product.
//
//   node e2e/shoot-analyses.mjs [baseURL] [project] [outDir]
//
// The project must already have been seeded by e2e/seed-analyses.mjs.
import { chromium } from '@playwright/test';

const base = process.argv[2] || 'http://127.0.0.1:8095';
const project = process.argv[3] || 'coffee-analyses';
const out = process.argv[4] || 'e2e';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });

// The library: what has been run, newest first, each addressable and any two diffable.
await page.goto(base + '/ui/projects/' + encodeURIComponent(project) + '/analyses', {
  waitUntil: 'networkidle',
  timeout: 60000,
});
await page.waitForTimeout(400);
await page.screenshot({ path: out + '/analyses-library.png', fullPage: true });

// One run: the record, its findings, and the evidence behind each one.
const runs = await page.locator('a.analysis-run').evaluateAll((links) =>
  links.map((link) => link.getAttribute('href')),
);
if (runs.length < 4) {
  throw new Error('expected four stored runs, saw ' + runs.length);
}
await page.goto(base + runs[0], { waitUntil: 'networkidle', timeout: 60000 });
await page.waitForTimeout(300);
await page.screenshot({ path: out + '/analyses-run.png', fullPage: true });

// The findings diff between the two coverage runs (the first run of each commit).
const before = runs[runs.length - 1].split('/').pop();
const after = runs[1].split('/').pop();
await page.goto(
  base +
    '/ui/projects/' +
    encodeURIComponent(project) +
    '/analyses/diff?from=' +
    encodeURIComponent(before) +
    '&to=' +
    encodeURIComponent(after),
  { waitUntil: 'networkidle', timeout: 60000 },
);
await page.waitForTimeout(300);
await page.screenshot({ path: out + '/analyses-diff.png', fullPage: true });

console.log(
  'captured',
  out + '/analyses-library.png,',
  out + '/analyses-run.png,',
  out + '/analyses-diff.png',
);
await browser.close();
