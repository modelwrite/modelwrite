// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Exports the scoped drawings from a server that ALREADY holds the coffee-machine corpus, at the
// commit it holds - so an export can be re-taken after a change without re-seeding. It writes each
// SVG into e2e/ and reports the viewBox, the byte-identity of a second fetch, and the label size
// each canvas lands at on a 1920x1080 slide. Usage:
//   node label-fix-export.mjs [baseURL] [project]

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = process.argv[2] || 'http://127.0.0.1:8096';
const PROJECT = process.argv[3] || 'coffee';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const CORPUS = path.join(
  HERE, '..', 'sample', 'corpus', 'coffee-machine', 'okf', 'expected', 'coffee_machine_model.json'
);

const count = (text, needle) => text.split(needle).length - 1;
const sha256 = (buffer) => crypto.createHash('sha256').update(buffer).digest('hex');

function viewBox(svg) {
  const match = /viewBox='([^']*)'/.exec(svg);
  const parts = match[1].split(/\s+/).map(Number);
  return { width: parts[2], height: parts[3] };
}

function caption(svg) {
  const match = /<text class='mw-caption'[^>]*>([^<]*)<\/text>/.exec(svg);
  return match ? match[1] : '(no caption)';
}

async function main() {
  const listed = await (await fetch(BASE + '/projects/' + PROJECT + '/commits?branch=main')).json();
  const hash = listed[0].hash;
  const corpus = JSON.parse(fs.readFileSync(CORPUS, 'utf8'));

  const degree = new Map(corpus.graph.nodes.map((n) => [n.id, 0]));
  for (const edge of corpus.graph.edges) {
    degree.set(edge.source, (degree.get(edge.source) || 0) + 1);
    degree.set(edge.target, (degree.get(edge.target) || 0) + 1);
  }
  const byDegree = corpus.graph.nodes
    .filter((n) => n.kind === 'block')
    .sort((a, b) => degree.get(b.id) - degree.get(a.id) || (a.id < b.id ? -1 : 1));
  const hub = byDegree[0].id;
  const typical = byDegree[Math.floor(byDegree.length / 2)].id;
  const holders = new Map();
  for (const edge of corpus.graph.edges) {
    if (edge.kind === 'part' || edge.kind === 'contains') {
      holders.set(edge.source, (holders.get(edge.source) || 0) + 1);
    }
  }
  const root = [...holders.keys()].sort(
    (a, b) => holders.get(b) - holders.get(a) || (a < b ? -1 : 1)
  )[0];
  const hood = (id, hops) =>
    'scope=neighbourhood&element=' + encodeURIComponent(id) + '&hops=' + hops;

  const scopes = [
    { file: 'svg-scope-full.svg', query: '' },
    { file: 'svg-scope-kinds-block-requirement.svg', query: 'scope=kinds&kinds=block,requirement' },
    { file: 'svg-scope-kinds-requirement.svg', query: 'scope=kinds&kinds=requirement' },
    { file: 'svg-scope-kinds-usecase.svg', query: 'scope=kinds&kinds=usecase' },
    { file: 'svg-scope-hood-1-hub.svg', query: hood(hub, 1) },
    { file: 'svg-scope-hood-2-hub.svg', query: hood(hub, 2) },
    { file: 'svg-scope-hood-2.svg', query: hood(typical, 2) },
    { file: 'svg-scope-containment.svg', query: 'scope=containment&element=' + encodeURIComponent(root) },
  ];

  console.log('commit ' + hash);
  for (const scope of scopes) {
    const url =
      BASE + '/ui/projects/' + PROJECT + '/diagram.svg?commit=' + hash + '&view=structure&' + scope.query;
    const first = await fetch(url);
    if (!first.ok) throw new Error(url + ' -> ' + first.status);
    const body = Buffer.from(await first.arrayBuffer());
    const again = Buffer.from(await (await fetch(url)).arrayBuffer());
    const svg = body.toString('utf8');
    const box = viewBox(svg);
    fs.writeFileSync(path.join(HERE, scope.file), body);
    const scale = Math.min(1920 / box.width, 1080 / box.height);
    console.log(
      '  ' + scope.file.padEnd(38) +
      ' viewBox ' + (box.width + 'x' + box.height).padEnd(12) +
      ' nodes ' + String(count(svg, "class='node'")).padEnd(4) +
      ' labels ' + String(count(svg, "class='edge-label'")).padEnd(4) +
      ' leaders ' + String(count(svg, "class='edge-leader'")).padEnd(4) +
      ' label ' + (13.0 * scale).toFixed(1).padEnd(6) + 'px' +
      ' identical ' + (sha256(body) === sha256(again))
    );
    console.log('      caption: ' + caption(svg));
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
