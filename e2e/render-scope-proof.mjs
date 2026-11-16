// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Renders the exported SVGs at SLIDE SCALE (1920x1080) and reads back what the browser COMPUTES:
// the colours that actually paint, and the label size that actually lands. Nothing here trusts
// the stylesheet text - the controller's complaint was found by looking at a rendered export, so
// the proof is taken from the same place.
//
// Usage: node render-scope-proof.mjs   (after scope-proof.mjs has saved the SVGs)

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const HERE = path.dirname(fileURLToPath(import.meta.url));

const FILES = [
  { svg: 'svg-scope-full.svg', png: 'svg-scope-full-slide.png', label: 'the whole model' },
  {
    svg: 'svg-scope-kinds-block-requirement.svg',
    png: 'svg-scope-kinds-block-requirement-slide.png',
    label: 'kinds block + requirement',
  },
  {
    svg: 'svg-scope-kinds-requirement.svg',
    png: 'svg-scope-kinds-requirement-slide.png',
    label: 'kind requirement',
  },
  {
    svg: 'svg-scope-hood-1-hub.svg',
    png: 'svg-scope-hood-1-hub-slide.png',
    label: '1 hop of the hub',
  },
  {
    svg: 'svg-scope-hood-2-hub.svg',
    png: 'svg-scope-hood-2-hub-slide.png',
    label: '2 hops of the hub',
  },
  { svg: 'svg-scope-hood-2.svg', png: 'svg-scope-hood-2-slide.png', label: '2 hops of a typical element' },
  {
    svg: 'svg-scope-containment.svg',
    png: 'svg-scope-containment-slide.png',
    label: 'a containment branch',
  },
  { svg: 'svg-scope-kinds-usecase.svg', png: 'svg-scope-kinds-usecase-slide.png', label: 'kind usecase' },
];

/** The retired brand blue, in the terms the browser reports colours. */
const RETIRED = ['rgb(37, 99, 235)', 'rgb(29, 78, 216)', 'rgb(139, 148, 158)', 'rgb(89, 99, 110)'];

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
  await page.setContent('<!doctype html><html><body style="margin:0;background:#fff"></body></html>');

  const rows = [];
  for (const file of FILES) {
    const raw = fs.readFileSync(path.join(HERE, file.svg), 'utf8');
    const markup = raw.replace(/^<\?xml[^>]*\?>\s*/, '');
    await page.evaluate((html) => {
      document.body.innerHTML = html;
    }, markup);
    const measured = await page.evaluate(() => {
      const svg = document.querySelector('svg');
      // The slide: the whole drawing fitted into one 1920x1080 frame.
      svg.removeAttribute('width');
      svg.removeAttribute('height');
      svg.setAttribute('style', 'width:1920px;height:1080px;display:block');
      const css = (selector) => {
        const el = document.querySelector(selector);
        return el ? getComputedStyle(el) : null;
      };
      const name = css('g.node .node-name');
      const blockRect = css("g.node[data-mw-kind='block'] .node-rect");
      const depEdge = css("g.edge[data-mw-kind='dependency'] .mw-edge");
      const accentArrow = css('polygon.arrow-accent');
      const node = document.querySelector('svg g.node .node-rect');
      const ctm = node ? node.getScreenCTM() : null;
      const fontSize = name ? parseFloat(name.fontSize) : 13;
      return {
        viewBox: svg.getAttribute('viewBox'),
        viewportPx: '1920x1080',
        ctmScale: ctm ? Number(ctm.a.toFixed(4)) : null,
        // What the browser actually renders for a node label at this scale.
        renderedLabelPx: ctm ? Number((fontSize * ctm.a).toFixed(2)) : null,
        nodeNameFill: name ? name.fill : null,
        nodeNameFontSize: name ? name.fontSize : null,
        blockFill: blockRect ? blockRect.fill : null,
        blockStroke: blockRect ? blockRect.stroke : null,
        dependencyEdgeStroke: depEdge ? depEdge.stroke : null,
        accentArrowFill: accentArrow ? accentArrow.fill : null,
        captionFill: css('.mw-caption') ? css('.mw-caption').fill : null,
        captionBandFill: css('.mw-caption-band') ? css('.mw-caption-band').fill : null,
        svgBackground: css('svg') ? css('svg').backgroundColor : null,
      };
    });
    await page.screenshot({ path: path.join(HERE, file.png) });
    rows.push({ ...file, ...measured });
  }
  await browser.close();

  const problems = [];
  for (const row of rows) {
    for (const value of [
      row.nodeNameFill,
      row.blockFill,
      row.blockStroke,
      row.dependencyEdgeStroke,
      row.accentArrowFill,
      row.captionFill,
    ]) {
      if (value && RETIRED.includes(value)) {
        problems.push(row.svg + ' paints the retired colour ' + value);
      }
    }
    if (row.blockStroke && row.dependencyEdgeStroke && row.blockStroke !== row.dependencyEdgeStroke) {
      problems.push(row.svg + ': the block stroke and the dependency edge are not the one accent');
    }
    if (row.captionBandFill && row.captionBandFill !== 'rgb(245, 247, 249)') {
      problems.push(row.svg + ': the caption band is not the surface-1 token');
    }
  }

  console.log(JSON.stringify(rows, null, 1));
  console.log('');
  console.log('COMPUTED-COLOUR CHECKS');
  console.log('  block node stroke:      ' + rows[0].blockStroke + '  (--kind-block / --accent #0b6e99)');
  console.log('  block node fill:        ' + rows[0].blockFill + '  (--kind-block-bg #e6f2f7)');
  console.log('  dependency edge stroke: ' + rows[0].dependencyEdgeStroke + '  (--accent #0b6e99)');
  console.log('  accent arrowhead fill:  ' + rows[0].accentArrowFill + '  (--accent #0b6e99)');
  console.log('  node name fill:         ' + rows[0].nodeNameFill + '  (--text #101418)');
  console.log('  caption band fill:      ' + rows[0].captionBandFill + '  (--surface-1 #f5f7f9)');
  console.log('  svg background:         ' + rows[0].svgBackground);
  console.log(
    problems.length ? '  FAILED: ' + problems.join('; ') : '  OK: no retired colour is painted anywhere'
  );

  console.log('');
  console.log('SLIDE-SCALE READABILITY (browser-measured, 1920x1080)');
  for (const row of rows) {
    console.log(
      '  ' + row.label.padEnd(28) + ' viewBox ' + row.viewBox.padEnd(14) + ' label ' +
        row.renderedLabelPx + ' px  -> ' + row.png
    );
  }
  if (problems.length) process.exitCode = 1;
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
