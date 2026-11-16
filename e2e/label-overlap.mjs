// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// MEASURES edge-label crowding in an exported diagram. It renders the SVG the way a slide does
// (1920x1080), reads back every edge label's real rendered box from the browser, and reports how
// many of them overlap another label, a node box, or a drawn edge line - with the area of the
// worst collisions. This is a measurement tool, not a fix: it is what tells us whether the
// crowding complaint is one bad case or a systematic placement bug.
//
// Usage: node label-overlap.mjs <input.svg> [more.svg ...]

import fs from 'node:fs';
import path from 'node:path';

const files = process.argv.slice(2);

function rectOverlap(a, b) {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

const browser = await import('playwright').then((m) => m.chromium.launch());
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');

const report = [];
for (const file of files) {
  const raw = fs.readFileSync(file, 'utf8').replace(/^<\?xml[^>]*\?>\s*/, '');
  await page.evaluate((html) => {
    document.body.innerHTML = html;
  }, raw);
  const data = await page.evaluate(() => {
    const svg = document.querySelector('svg');
    svg.removeAttribute('width');
    svg.removeAttribute('height');
    svg.setAttribute('style', 'width:1920px;height:1080px;display:block');
    const root = document.querySelector('svg g.mw-transform');
    const scale = root.getScreenCTM().a;
    // Every edge group, in draw order: the identity a label and a line are matched on. Two edges
    // can carry the same text, so comparing labels by their text would mis-attribute a crossing.
    const groups = [...document.querySelectorAll('g.edge')];
    const labels = [];
    groups.forEach((g, edge) => {
      const t = g.querySelector('text.edge-label');
      if (!t) return;
      const b = t.getBBox();
      labels.push({
        edge,
        text: t.textContent,
        source: g.getAttribute('data-mw-source'),
        target: g.getAttribute('data-mw-target'),
        kind: g.getAttribute('data-mw-kind'),
        x: b.x, y: b.y, w: b.width, h: b.height,
      });
    });
    const boxes = [];
    for (const g of document.querySelectorAll('g.node')) {
      const r = g.querySelector('rect.node-rect');
      if (!r) continue;
      boxes.push({
        id: g.getAttribute('data-mw-id'),
        name: g.getAttribute('data-mw-name'),
        x: parseFloat(r.getAttribute('x')), y: parseFloat(r.getAttribute('y')),
        w: parseFloat(r.getAttribute('width')), h: parseFloat(r.getAttribute('height')),
      });
    }
    // Every drawn edge line, sampled from the browser's own geometry so a jump arc is included.
    const lines = [];
    groups.forEach((g, edge) => {
      const el = g.querySelector('polyline.mw-edge, path.mw-edge, line.mw-edge');
      if (!el) return;
      const total = el.getTotalLength();
      const n = Math.max(8, Math.min(400, Math.ceil(total / 4)));
      const pts = [];
      for (let i = 0; i <= n; i += 1) {
        const p = el.getPointAtLength((total * i) / n);
        pts.push([p.x, p.y]);
      }
      lines.push({ edge, pts });
    });
    return { scale, labels, boxes, lines };
  });

  const { scale, labels, boxes, lines } = data;
  const labelLabel = [];
  const labelBox = [];
  const labelLine = [];
  for (let i = 0; i < labels.length; i += 1) {
    for (let j = i + 1; j < labels.length; j += 1) {
      const area = rectOverlap(labels[i], labels[j]);
      if (area > 0) labelLabel.push({ a: labels[i].text, b: labels[j].text, area });
    }
    for (const box of boxes) {
      const area = rectOverlap(labels[i], box);
      if (area > 0) labelBox.push({ label: labels[i].text, box: box.name, area });
    }
    // A label whose box crosses a drawn edge line: is it crossing its OWN line (which is
    // normal - a label sits beside its line) or somebody else's?
    let own = 0;
    let other = 0;
    let deepest = 0;
    let where = null;
    for (const line of lines) {
      for (const p of line.pts) {
        if (!(p[0] > labels[i].x && p[0] < labels[i].x + labels[i].w &&
              p[1] > labels[i].y && p[1] < labels[i].y + labels[i].h)) continue;
        // How far INSIDE the box the line runs. A line that only grazes the boundary is not
        // struck through the text: the ink of a 15-unit box sits inside its own boundary.
        const depth = Math.min(
          p[0] - labels[i].x,
          labels[i].x + labels[i].w - p[0],
          p[1] - labels[i].y,
          labels[i].y + labels[i].h - p[1]
        );
        if (line.edge === labels[i].edge) { own += 1; continue; }
        other += 1;
        if (!where) where = 'edge ' + line.edge + ' at ' + p.map((v) => v.toFixed(2)).join(',');
        deepest = Math.max(deepest, depth);
      }
    }
    if (other > 0) {
      labelLine.push({
        label: labels[i].text, other_lines: other, own_lines: own, where,
        deepest: Number(deepest.toFixed(2)),
      });
    }
  }

  const fmt = (rows, key) =>
    rows
      .slice()
      .sort((a, b) => b.area - a.area)
      .slice(0, 6)
      .map((r) => (key === 'line'
        ? '      ' + r.label + ' is entered by ' + r.other_lines + " other edge's line(s), " +
          r.deepest + ' u inside (' + r.where + ')'
        : '      ' + (r.label || r.a) + (r.box ? '  over box ' + r.box : '  over label ' + r.b) +
          '  ' + r.area.toFixed(1) + ' u2 (' + (r.area * scale * scale).toFixed(1) + ' px2)'))
      .join('\n');

  report.push(
    path.basename(file) + '  (slide scale ' + scale.toFixed(3) + ')' +
    '\n    edge labels drawn:              ' + labels.length +
    '\n    label-label overlaps:           ' + labelLabel.length +
    '\n    label-box overlaps:             ' + labelBox.length +
    '\n    labels their box is entered by another edge: ' +
      labelLine.filter((r) => r.deepest >= 0.25).length +
    '\n    labels only grazed at the boundary (<0.25 u): ' +
      labelLine.filter((r) => r.deepest < 0.25).length +
    (labelLabel.length ? '\n    worst label-label:\n' + fmt(labelLabel) : '') +
    (labelBox.length ? '\n    worst label-box:\n' + fmt(labelBox) : '') +
    (labelLine.length ? '\n    worst crossings:\n' + fmt(labelLine, 'line') : '')
  );
}

await browser.close();
console.log(report.join('\n\n'));
