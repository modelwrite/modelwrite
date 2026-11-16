// Render the exported live SVG to PNG so a human (or the controller) can LOOK at it.
// Used once for the idc-1 trial redeploy verification.
import { chromium } from 'playwright';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const wrapper = path.resolve('svg-live-preview.html');
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
page.setDefaultTimeout(120000);
await page.goto(pathToFileURL(wrapper).href, { waitUntil: 'load' });
await page.waitForFunction(() => {
  const i = document.getElementById('d');
  return i && i.complete && i.naturalWidth > 0;
}, null, { timeout: 60000 });

const info = await page.evaluate(() => {
  const i = document.getElementById('d');
  const r = i.getBoundingClientRect();
  return { naturalW: i.naturalWidth, naturalH: i.naturalHeight, renderedW: Math.round(r.width), renderedH: Math.round(r.height) };
});

const el = await page.$('#d');
await el.screenshot({ path: path.resolve('svg-live-render.png') });

// A detail crop: top-left quadrant at readable scale (boxes + labels + arrowheads).
await page.screenshot({ path: path.resolve('svg-live-render-detail.png'), clip: { x: 0, y: 0, width: 700, height: 560 } });

await browser.close();
console.log('RENDERED', JSON.stringify(info));
