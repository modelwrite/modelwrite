// SPDX-License-Identifier: AGPL-3.0-or-later
// Regenerates the Analytics section listing in website/index.html from
// spec/analytics/metric_definitions.json, the engine's authoritative schema.
// The site is static HTML/CSS; this script is the only place the listing is
// generated, so the listing is never a hand-maintained copy.
//
// Usage: node website/generate-analytics.mjs   (prints the <ul class="metrics"> block)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const jsonPath = path.resolve(here, '..', 'spec', 'analytics', 'metric_definitions.json');
const metrics = JSON.parse(fs.readFileSync(jsonPath, 'utf8'));

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

const rows = metrics.map((m) => {
  const deps = (m.depends_on || []).map((d) => '<code>' + esc(d) + '</code>').join(', ');
  return [
    '    <li class="metric">',
    '      <div class="metric__head">',
    '        <h4>' + esc(m.name) + '</h4>',
    '        <code class="metric__id">' + esc(m.id) + '</code>',
    '        <span class="metric__status">' + esc(m.status) + '</span>',
    '      </div>',
    '      <p class="metric__desc">' + esc(m.description) + '</p>',
    '      <p class="metric__deps"><span>depends on</span> ' + deps + '</p>',
    '    </li>',
  ].join('\n');
}).join('\n');

console.log(rows);
