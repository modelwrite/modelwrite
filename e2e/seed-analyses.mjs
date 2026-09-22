// SPDX-License-Identifier: AGPL-3.0-or-later
// Seeds a scratch server with a REAL model, two analyses, a real model change and two more
// analyses, so the analyses library and the findings diff can be looked at. Dev-only; never
// part of the product.
//
//   node e2e/seed-analyses.mjs [baseURL] [project]
//
// The model is the coffee-machine corpus (a real MagicDraw export) and its corrupted sibling,
// which is a real committed change: it drops a requirement and three of its links. The change
// is committed; the analyses are then run again, so the diff is between two COMMITS.
import fs from 'fs';

const base = process.argv[2] || 'http://127.0.0.1:8095';
const project = process.argv[3] || 'coffee-analyses';

const expected = JSON.parse(
  fs.readFileSync('sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json', 'utf8'),
);
const corrupted = JSON.parse(
  fs.readFileSync('sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json', 'utf8'),
);

// The changed model: the corrupted fixture (a real, committed change to the export) plus a
// node with no edges and a requirement no link covers, so the change is visible in both
// analyses.
const changed = JSON.parse(JSON.stringify(corrupted));
changed.graph.nodes.push({
  id: 'a1-e2e-orphan',
  kind: 'block',
  name: 'Orphan added by the A1 proof',
});
changed.requirements.push({
  id: 'a1-e2e-requirement',
  name: 'A requirement added by the A1 proof',
  reqText: 'The A1 proof declares a requirement no link covers.',
});

async function postJson(path, body) {
  const response = await fetch(base + path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error('POST ' + path + ' -> ' + response.status + ' ' + text.slice(0, 400));
  }
  return JSON.parse(text);
}

async function commit(project, branch, message, okf) {
  const commit = await postJson('/projects/' + project + '/commits', {
    branch,
    author: 'alex',
    message,
    okf,
  });
  return commit.hash;
}

/// Run one analysis through the PAGE, and return the address of the record it stored.
async function run(project, definition, branch) {
  const response = await fetch(
    base + '/ui/projects/' + encodeURIComponent(project) + '/analyses',
    {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ definition, branch }).toString(),
      redirect: 'manual',
    },
  );
  const location = response.headers.get('location');
  if (!location) {
    throw new Error(
      'running ' + definition + ' -> ' + response.status + ' ' + (await response.text()).slice(0, 400),
    );
  }
  return location;
}

const created = await fetch(base + '/projects', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ name: project }),
});
if (!created.ok && created.status !== 409) {
  throw new Error('creating the project -> ' + created.status + ' ' + (await created.text()).slice(0, 300));
}

const before = await commit(project, 'main', 'the exported model', expected);
const beforeRuns = [
  await run(project, 'requirement-coverage', 'main'),
  await run(project, 'model-health', 'main'),
];

const after = await commit(project, 'main', 'drop a requirement and add a gap', changed);
const afterRuns = [
  await run(project, 'requirement-coverage', 'main'),
  await run(project, 'model-health', 'main'),
];

console.log(JSON.stringify({
  project,
  before,
  after,
  coverageDiff: afterRuns[0] + '/../diff?from=' + encodeURIComponent(beforeRuns[0].split('/').pop()) + '&to=' + encodeURIComponent(afterRuns[0].split('/').pop()),
  beforeRuns,
  afterRuns,
}, null, 2));
