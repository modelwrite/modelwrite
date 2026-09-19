// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Seeds a running mw-server with the coffee-machine corpus, over the SAME JSON
// API the Rust corpus test exercises (server/tests/corpus.rs). The fixture is
// the exact path test_support::load_okf_expected() reads, so the browser test's
// model is byte-identical to the one the Rust tests gate.

const fs = require('node:fs');
const path = require('node:path');
const { repoRoot, baseURL: defaultBaseURL } = require('./env');

const PROJECT = 'coffee';
const CORPUS = path.join(
  repoRoot,
  'sample', 'corpus', 'coffee-machine', 'okf', 'expected',
  'coffee_machine_model.json'
);

function corpusOkf() {
  return JSON.parse(fs.readFileSync(CORPUS, 'utf8'));
}

async function post(url, body) {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}: ${await response.text()}`);
  }
  return response;
}

async function seed(baseURL = defaultBaseURL) {
  await post(`${baseURL}/projects`, { name: PROJECT });
  const commitResponse = await post(`${baseURL}/projects/${PROJECT}/commits`, {
    branch: 'main',
    author: 'e2e',
    message: 'seed the coffee-machine corpus',
    okf: corpusOkf(),
  });
  const commit = await commitResponse.json();
  return { project: PROJECT, hash: commit.hash };
}

module.exports = { seed, corpusOkf, PROJECT, CORPUS };

// Standalone: node seed.js [baseURL]
if (require.main === module) {
  seed(process.argv[2] || defaultBaseURL)
    .then(({ project, hash }) => console.log(`seeded ${project} at commit ${hash}`))
    .catch((error) => {
      console.error(error.message);
      process.exitCode = 1;
    });
}
