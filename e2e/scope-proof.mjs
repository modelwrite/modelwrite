// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// PROOF for the scoped export: seed a scratch server with the coffee-machine corpus, export the
// SAME model at full scope and at scoped levels, save every SVG into e2e/, and report what each
// one is - node/edge counts, viewBox, byte-identity, and the label size it lands at on a slide.
//
// Usage: node scope-proof.mjs [baseURL]        (default http://127.0.0.1:8097)

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = process.argv[2] || 'http://127.0.0.1:8097';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const CORPUS = path.join(
  HERE, '..', 'sample', 'corpus', 'coffee-machine', 'okf', 'expected', 'coffee_machine_model.json'
);

const PROJECT = 'coffee';

async function post(url, body) {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(response.status + ' ' + response.statusText + ': ' + (await response.text()));
  }
  return response;
}

async function seed() {
  await post(BASE + '/projects', { name: PROJECT });
  const committed = await post(BASE + '/projects/' + PROJECT + '/commits', {
    branch: 'main',
    author: 'scope-proof',
    message: 'seed the coffee-machine corpus',
    okf: JSON.parse(fs.readFileSync(CORPUS, 'utf8')),
  });
  const body = await committed.json();
  return body.hash;
}

const count = (text, needle) => text.split(needle).length - 1;
const sha256 = (buffer) => crypto.createHash('sha256').update(buffer).digest('hex');

function viewBox(svg) {
  const match = /viewBox='([^']*)'/.exec(svg);
  const parts = match[1].split(/\s+/).map(Number);
  return { width: parts[2], height: parts[3] };
}

/** The size a node label lands at when the drawing is fitted into one 1920x1080 slide. */
function slideLabelPx(box) {
  return 13.0 * Math.min(1920 / box.width, 1080 / box.height);
}

function caption(svg) {
  const match = /<text class='mw-caption'[^>]*>([^<]*)<\/text>/.exec(svg);
  return match ? match[1] : '(no caption)';
}

function provenance(svg) {
  const match = /<!-- modelwrite-provenance: ([^>]*) -->/.exec(svg);
  return match ? match[1].trim() : '(no provenance)';
}

/** The degrees, the most connected block, and the container that holds the most. */
function analyse(corpus) {
  const nodes = corpus.graph.nodes;
  const degree = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of corpus.graph.edges) {
    degree.set(edge.source, (degree.get(edge.source) || 0) + 1);
    degree.set(edge.target, (degree.get(edge.target) || 0) + 1);
  }
  const name = (id) => {
    const node = nodes.find((candidate) => candidate.id === id);
    return node && node.name ? node.name : id;
  };
  const blocks = nodes.filter((node) => node.kind === 'block');
  const byDegree = [...blocks].sort(
    (a, b) => degree.get(b.id) - degree.get(a.id) || (a.id < b.id ? -1 : 1)
  );
  const holders = new Map();
  for (const edge of corpus.graph.edges) {
    if (edge.kind === 'part' || edge.kind === 'contains') {
      holders.set(edge.source, (holders.get(edge.source) || 0) + 1);
    }
  }
  const root = [...holders.keys()].sort(
    (a, b) => holders.get(b) - holders.get(a) || (a < b ? -1 : 1)
  )[0];
  return {
    degree,
    name,
    hub: byDegree[0].id,
    typical: byDegree[Math.floor(byDegree.length / 2)].id,
    root,
  };
}

async function main() {
  const hash = await seed();
  const corpus = JSON.parse(fs.readFileSync(CORPUS, 'utf8'));
  const info = analyse(corpus);
  const hood = (id, hops) =>
    'scope=neighbourhood&element=' + encodeURIComponent(id) + '&hops=' + hops;

  const scopes = [
    { file: 'svg-scope-full.svg', label: 'the whole model', query: '' },
    {
      file: 'svg-scope-kinds-block-requirement.svg',
      label: 'kinds block + requirement',
      query: 'scope=kinds&kinds=block,requirement',
    },
    {
      file: 'svg-scope-kinds-requirement.svg',
      label: 'kind requirement',
      query: 'scope=kinds&kinds=requirement',
    },
    {
      file: 'svg-scope-kinds-usecase.svg',
      label: 'kind usecase',
      query: 'scope=kinds&kinds=usecase',
    },
    {
      file: 'svg-scope-hood-1-hub.svg',
      label: '1 hop of the hub (' + info.name(info.hub) + ', degree ' + info.degree.get(info.hub) + ')',
      query: hood(info.hub, 1),
    },
    {
      file: 'svg-scope-hood-2-hub.svg',
      label: '2 hops of the hub (' + info.name(info.hub) + ')',
      query: hood(info.hub, 2),
    },
    {
      file: 'svg-scope-hood-2.svg',
      label: '2 hops of a typical element (' + info.name(info.typical) + ')',
      query: hood(info.typical, 2),
    },
    {
      file: 'svg-scope-containment.svg',
      label: 'containment branch of ' + info.name(info.root),
      query: 'scope=containment&element=' + encodeURIComponent(info.root),
    },
  ];

  const rows = [];
  for (const scope of scopes) {
    const url =
      BASE + '/ui/projects/' + PROJECT + '/diagram.svg?commit=' + hash + '&view=structure&' + scope.query;
    const first = await fetch(url);
    if (!first.ok) throw new Error(url + ' -> ' + first.status + ' ' + (await first.text()));
    const body = Buffer.from(await first.arrayBuffer());
    const again = Buffer.from(await (await fetch(url)).arrayBuffer());
    const svg = body.toString('utf8');
    const box = viewBox(svg);
    fs.writeFileSync(path.join(HERE, scope.file), body);
    rows.push({
      file: scope.file,
      label: scope.label,
      bytes: body.length,
      sha: sha256(body).slice(0, 16),
      identical: sha256(body) === sha256(again),
      viewBox: '0 0 ' + box.width + ' ' + box.height,
      nodes: count(svg, "class='node'"),
      edges: count(svg, "class='edge'"),
      arrows: count(svg, "class='arrow-"),
      dangling: count(svg, "class='dangling'"),
      labelPx: slideLabelPx(box).toFixed(1),
      provenance: provenance(svg),
      caption: caption(svg),
    });
  }

  console.log('commit ' + hash);
  console.log('');
  for (const row of rows) {
    console.log(row.file);
    console.log(
      '  ' + row.label + ': ' + row.nodes + ' nodes, ' + row.edges + ' edges, ' + row.arrows +
        ' arrowheads, ' + row.dangling + ' dangling, viewBox ' + row.viewBox
    );
    console.log(
      '  ' + row.bytes + ' bytes, sha256 ' + row.sha + ', re-export identical: ' + row.identical +
        ', label ' + row.labelPx + ' px at slide scale'
    );
    console.log('  provenance: ' + row.provenance);
    console.log('  caption:    ' + row.caption);
    console.log('');
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
