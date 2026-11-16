// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Renders an exported SVG at SLIDE SCALE (1920x1080) and reads back what the browser computes: the
// viewBox, the scale the drawing lands at, and the label size that actually paints. Usage:
//   node layout-render.mjs <input.svg> <output.png>

import fs from 'node:fs';
import { chromium } from 'playwright';

const [svgPath, pngPath] = process.argv.slice(2);
const raw = fs.readFileSync(svgPath, 'utf8').replace(/^<\?xml[^>]*\?>\s*/, '');
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');
await page.evaluate((html) => {
  document.body.innerHTML = html;
}, raw);
const measured = await page.evaluate(() => {
  const svg = document.querySelector('svg');
  svg.removeAttribute('width');
  svg.removeAttribute('height');
  svg.setAttribute('style', 'width:1920px;height:1080px;display:block');
  const name = document.querySelector('g.node .node-name');
  const rect = document.querySelector('svg g.node .node-rect');
  const ctm = rect.getScreenCTM();
  const box = rect.getBoundingClientRect();
  const drawing = document.querySelector('svg g.mw-transform').getBoundingClientRect();
  return {
    viewBox: svg.getAttribute('viewBox'),
    ctmScale: Number(ctm.a.toFixed(4)),
    renderedLabelPx: Number((parseFloat(getComputedStyle(name).fontSize) * ctm.a).toFixed(2)),
    nodePx: Number(box.width.toFixed(1)) + 'x' + Number(box.height.toFixed(1)),
    // how much of the 1920x1080 slide frame the drawing itself covers
    frameFillPercent: Number(((drawing.width * drawing.height) / (1920 * 1080) * 100).toFixed(1)),
    drawingPx: Number(drawing.width.toFixed(0)) + 'x' + Number(drawing.height.toFixed(0)),
  };
});
await page.screenshot({ path: pngPath });
console.log(JSON.stringify(measured));
await browser.close();
