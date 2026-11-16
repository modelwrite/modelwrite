// Probe: what colour is a dependency arrowhead ACTUALLY painted, and which arrow classes exist?
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const svg = fs.readFileSync(path.join(HERE, 'svg-scope-full.svg'), 'utf8');
const classes = [...new Set([...svg.matchAll(/class='(arrow-[a-z]+)'/g)].map((m) => m[1]))];
const style = /<style>([\s\S]*?)<\/style>/.exec(svg)[1];
const styled = classes.map((name) => ({ name, rule: style.includes('.' + name + ' ') }));
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.evaluate((html) => { document.body.innerHTML = html; }, svg.replace(/^<\?xml[^>]*\?>\s*/, ''));
const computed = await page.evaluate(() =>
  ['arrow-dependency', 'arrow-containment', 'arrow-flow', 'arrow-neutral'].map((name) => {
    const el = document.querySelector('polygon.' + name);
    return { name, fill: el ? getComputedStyle(el).fill : 'not drawn' };
  })
);
await browser.close();
console.log('arrow classes emitted: ' + JSON.stringify(classes));
console.log('styled by the exported stylesheet: ' + JSON.stringify(styled));
console.log('COMPUTED arrowhead fills: ' + JSON.stringify(computed, null, 1));
