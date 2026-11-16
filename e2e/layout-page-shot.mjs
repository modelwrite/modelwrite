// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Looks at the PAGE's diagram (which shares the renderer with the export) and reports what the
// browser drew: node boxes, the camera window, and the label size actually painted. Usage:
//   node layout-page-shot.mjs <pageURL> <out.png>

import { chromium } from 'playwright';

const [url, png] = process.argv.slice(2);
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(url, { waitUntil: 'networkidle' });
await page.waitForSelector('svg.mw-diagram-svg g.node', { timeout: 15000 });
const seen = await page.evaluate(() => {
  const svg = document.querySelector('svg.mw-diagram-svg');
  const rect = document.querySelector('svg g.node .node-rect');
  const ctm = rect.getScreenCTM();
  const vb = svg.viewBox.baseVal;
  return {
    nodes: document.querySelectorAll('svg g.node').length,
    edges: document.querySelectorAll('svg g.edge').length,
    viewBox: vb.width + 'x' + vb.height,
    labelPx: Number((parseFloat(getComputedStyle(document.querySelector('g.node .node-name')).fontSize) * ctm.a).toFixed(2)),
    captionPx: (document.querySelector('.mw-diagram-meta') || {}).textContent || null,
  };
});
await page.screenshot({ path: png, fullPage: false });
console.log(JSON.stringify(seen));
await browser.close();
