// SPDX-License-Identifier: AGPL-3.0-or-later
// Regenerates the example-corpus listing in website/examples.html from
// sample/examples/README.md, the authoritative licence table. The site is
// static HTML/CSS; this script is the only place the listing is generated, so
// the licence table can never drift from the page.
//
// The LOADABLE / READ-ONLY / NOT-IMPORTABLE state of each example is NOT in the
// README (its "what is importable" section predates mw-binding-sysmlv2). It is
// pinned here, keyed by example path, and grounded in the engine's current
// bindings: binding-xmi (SysML v1 XMI, round-trippable) and binding-sysmlv2
// (SysML v2 textual, Direction::ImportOnly viewer). A bundled example whose
// path has no entry here fails the script rather than rendering a blank state.
//
// Usage: node website/generate-examples.mjs   (prints the listing fragment)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const readmePath = path.resolve(here, '..', 'sample', 'examples', 'README.md');
const readme = fs.readFileSync(readmePath, 'utf8');

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
// Markdown **bold** -> <strong> (the only markdown the table cells use).
const md = (s) => esc(s).replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');

// ---------------------------------------------------------------------------
// The loadability truth, pinned to the engine's current bindings. This is the
// one piece the README does not carry (and must not drift from the page).
// ---------------------------------------------------------------------------
const STATE_DEFS = {
  loadable: {
    label: 'Loadable',
    css: 'pill--pass',
    note: 'XMI. The platform imports it today and writes it back — a round-trip, not a screenshot.',
  },
  viewer: {
    label: 'Read-only &middot; viewer',
    css: 'pill--warn',
    note: 'SysML v2 textual. Carried by mw-binding-sysmlv2 as Direction::ImportOnly — it can be read, not written back.',
  },
  refused: {
    label: 'Large &middot; refused pending acceptance',
    css: 'pill--warn',
    note: 'XMI (the loadable format), but 36&nbsp;MB and its migration needs 48,553 blocking losses (45,725 unmappable + 2,828 lossy) accepted by hand before it loads.',
  },
  'not-importable': {
    label: 'Not importable',
    css: 'pill--muted',
    note: "Gaphor's own .gaphor XML, not OMG XMI. No Gaphor binding exists today, so the reader cannot open it.",
  },
};

const EXAMPLE_STATE = {
  'sysml-v1/openmbee-tmt': 'refused',
  'sysml-v1/gaphor-examples': 'not-importable',
  'sysml-v2/gfse-models': 'viewer',
  'sysml-v2/mbse4u-batmobile': 'viewer',
  'sysml-v2/airbus-apollo-11': 'viewer',
};

// Cosmetic display titles (the path stays the canonical name below the title).
const TITLES = {
  'sysml-v1/openmbee-tmt': 'Thirty Meter Telescope',
  'sysml-v1/gaphor-examples': 'Gaphor examples',
  'sysml-v2/gfse-models': 'GfSE community models',
  'sysml-v2/mbse4u-batmobile': 'The Batmobile',
  'sysml-v2/airbus-apollo-11': 'Apollo 11 mission',
  'sysml-v2/omg-sysml-v2-release': 'OMG SysML v2 release',
  'arcadia-capella/eclipse-capella-samples': 'Capella samples',
  'arcadia-capella/dbinfrago-ife-variant': 'Capella IFE variant',
  'sysml-v2/mbse4u-sysmlv2-book': 'MBSE4U SysML v2 book examples',
};

// ---------------------------------------------------------------------------
// Parse the two markdown tables ("Bundled examples" and "Not bundled").
// ---------------------------------------------------------------------------
const lines = readme.split(/\r?\n/);
const tableStarts = [];
lines.forEach((l, i) => { if (/^\| Example \|/.test(l.trim())) tableStarts.push(i); });
if (tableStarts.length !== 2) {
  throw new Error('expected two "| Example |" tables in ' + readmePath + ', got ' + tableStarts.length);
}

function parseTable(start) {
  const rows = [];
  let i = start + 2; // skip header + separator
  while (i < lines.length && lines[i].trim().startsWith('|')) {
    const cells = lines[i].split('|').slice(1, -1).map((c) => c.trim());
    if (cells.length && cells[0]) rows.push(cells);
    i++;
  }
  return rows;
}

const bundled = parseTable(tableStarts[0]);    // [name, upstream, commit, licence, size, demonstrates]
const notBundled = parseTable(tableStarts[1]); // [name, upstream, commit, licence, why]

// Guard: every bundled example must have a pinned state, or the README drifted.
for (const row of bundled) {
  if (!(row[0] in EXAMPLE_STATE)) {
    throw new Error('no loadability state pinned for bundled example ' + row[0] + ' in ' + readmePath);
  }
}

// ---------------------------------------------------------------------------
// Render.
// ---------------------------------------------------------------------------
const stateKey = Object.entries(STATE_DEFS).map(([key, def]) => [
  '      <li>',
  '        <span class="pill ' + def.css + '">' + def.label + '</span>',
  '        <p>' + def.note + '</p>',
  '      </li>',
].join('\n')).join('\n');

const bundledRows = bundled.map((row) => {
  const [name, upstream, commit, licence, size, demonstrates] = row;
  const state = STATE_DEFS[EXAMPLE_STATE[name]];
  return [
    '      <tr>',
    '        <td><span class="ex-name">' + md(TITLES[name] || name) + '</span>',
    '            <code class="ex-path">' + esc(name) + '</code>',
    '            <span class="ex-meta">commit ' + esc(commit) + ' &middot; ' + esc(size) + '</span></td>',
    '        <td><code class="ex-upstream">' + esc(upstream) + '</code></td>',
    '        <td>' + md(licence) + '</td>',
    '        <td class="ex-demo">' + md(demonstrates) + '</td>',
    '        <td><span class="pill ' + state.css + '">' + state.label + '</span></td>',
    '      </tr>',
  ].join('\n');
}).join('\n');

const notBundledRows = notBundled.map((row) => {
  const [name, upstream, commit, licence, why] = row;
  return [
    '      <tr>',
    '        <td><span class="ex-name">' + md(TITLES[name] || name) + '</span>',
    '            <code class="ex-path">' + esc(name) + '</code>',
    '            <span class="ex-meta">commit ' + esc(commit) + '</span></td>',
    '        <td><code class="ex-upstream">' + esc(upstream) + '</code></td>',
    '        <td>' + md(licence) + '</td>',
    '        <td class="ex-demo">' + md(why) + '</td>',
    '      </tr>',
  ].join('\n');
}).join('\n');

const out = [
  '<ul class="examples-key">',
  stateKey,
  '</ul>',
  '',
  '<h3 class="examples-sub">Bundled &mdash; present in this repository</h3>',
  '<div class="table-scroll">',
  '<table class="examples">',
  '  <thead>',
  '    <tr>',
  '      <th scope="col">Example</th>',
  '      <th scope="col">Upstream repo</th>',
  '      <th scope="col">Licence</th>',
  '      <th scope="col">What it demonstrates</th>',
  '      <th scope="col">State today</th>',
  '    </tr>',
  '  </thead>',
  '  <tbody>',
  bundledRows,
  '  </tbody>',
  '</table>',
  '</div>',
  '',
  '<h3 class="examples-sub">Not bundled &mdash; fetch with the script</h3>',
  '<div class="table-scroll">',
  '<table class="examples">',
  '  <thead>',
  '    <tr>',
  '      <th scope="col">Example</th>',
  '      <th scope="col">Upstream repo</th>',
  '      <th scope="col">Licence</th>',
  '      <th scope="col">Why it is not bundled</th>',
  '    </tr>',
  '  </thead>',
  '  <tbody>',
  notBundledRows,
  '  </tbody>',
  '</table>',
  '</div>',
].join('\n');

console.log(out);
