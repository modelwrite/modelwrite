// SPDX-License-Identifier: AGPL-3.0-or-later
// Seeds a scratch server with the no-hazards STPA model, so the not-started state can be looked
// at (and screenshotted). Dev-only; never part of the product.
//
//   node e2e/seed-stpa-not-started.mjs [baseURL] [project]
import fs from 'fs';

const base = process.argv[2] || 'http://127.0.0.1:8094';
const project = process.argv[3] || 'stpa-no-hazards';
const model = JSON.parse(fs.readFileSync('sample/stpa/no-hazards.json', 'utf8'));

const created = await fetch(base + '/projects', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ name: project }),
});
if (!created.ok && created.status !== 409) {
  throw new Error('creating the project -> ' + created.status + ' ' + (await created.text()).slice(0, 300));
}
const committed = await fetch(base + '/projects/' + encodeURIComponent(project) + '/commits', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ branch: 'main', author: 'alex', message: 'no hazards', okf: model }),
});
const text = await committed.text();
if (!committed.ok) throw new Error('committing -> ' + committed.status + ' ' + text.slice(0, 400));
console.log('seeded', project, JSON.parse(text).hash);
