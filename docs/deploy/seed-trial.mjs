// Reproducible seed for the modelwrite trial baseline.
//
// Rebuilds the seven seeded models (with cafe-stand's two branches and their
// crossModelEdges, plus the defective fire-suppression STPA example) from a FRESH
// checkout of this repository, reading the model sources directly from
// e2e/models/, sample/corpus/ and sample/stpa/. The commit hashes are
// content-addressed and deterministic, so this reproduces the exact baseline the
// hourly reset restores - no captured snapshot needed.
//
// Usage (against a running mw-server, e.g. a throwaway seed container):
//   MW_BASE_URL=http://127.0.0.1:3199 MW_TOKEN=<token> node docs/deploy/seed-trial.mjs
//
// In configured (static) auth mode the commit author must be the verified
// subject ("admin"); a non-matching author is refused 403.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');
const BASE = process.env.MW_BASE_URL || 'http://127.0.0.1:3102';
const TOKEN = process.env.MW_TOKEN || '';

const headers = {
  'content-type': 'application/json',
  'authorization': 'Bearer ' + TOKEN,
};

const readModel = (rel) => JSON.parse(fs.readFileSync(path.join(repoRoot, rel), 'utf8'));

const SOURCES = {
  'coffee-machine': 'sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json',
  'sandwich-toaster': 'e2e/models/sandwich-toaster.json',
  'purchasing-terminal': 'e2e/models/purchasing-terminal.json',
  'floor-robot': 'e2e/models/floor-robot.json',
  'microduck': 'e2e/models/microduck.json',
  'cafe-stand': 'e2e/models/cafe-stand.json',
  'fire-suppression': 'sample/stpa/fire-suppression-defective.json',
};

async function post(url, body) {
  const res = await fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw new Error(res.status + ' ' + url + ' :: ' + text);
  try { return JSON.parse(text); } catch { return text; }
}

async function main() {
  const order = ['coffee-machine', 'sandwich-toaster', 'purchasing-terminal',
                 'floor-robot', 'microduck', 'fire-suppression', 'cafe-stand'];
  for (const p of order) {
    try { await post(BASE + '/projects', { name: p }); console.log('project', p); }
    catch (e) { if (!e.message.includes('409')) throw e; console.log('project exists', p); }
  }

  const commit = async (project, okf) =>
    (await post(BASE + '/projects/' + project + '/commits', {
      branch: 'main', author: 'admin', message: 'seed platform scenario', okf,
    })).hash;

  const hashes = {
    'coffee-machine': await commit('coffee-machine', readModel(SOURCES['coffee-machine'])),
    'sandwich-toaster': await commit('sandwich-toaster', readModel(SOURCES['sandwich-toaster'])),
    'purchasing-terminal': await commit('purchasing-terminal', readModel(SOURCES['purchasing-terminal'])),
    'floor-robot': await commit('floor-robot', readModel(SOURCES['floor-robot'])),
    'microduck': await commit('microduck', readModel(SOURCES['microduck'])),
    'fire-suppression': await commit('fire-suppression', readModel(SOURCES['fire-suppression'])),
  };
  console.log('hashes', JSON.stringify(hashes));

  // cafe-stand main: re-pin each reference to the actual commit hash, and give the
  // floor-care reference its CS-4 satisfiedBy edge to the floor-robot collector.
  const cafeMain = readModel(SOURCES['cafe-stand']);
  for (const ref of cafeMain.references) {
    ref.revision = hashes[ref.project];
    if (ref.role === 'floor-care') {
      ref.crossModelEdges = [{ from: 'req-floor-clear', relation: 'satisfiedBy', to: 'collector' }];
    }
  }
  const mainHash = (await post(BASE + '/projects/cafe-stand/commits', {
    branch: 'main', author: 'admin', message: 'seed platform scenario', okf: cafeMain,
  })).hash;
  console.log('cafe-stand main', mainHash);

  await post(BASE + '/projects/cafe-stand/branches', { name: 'with-microduck', from: mainHash });

  // with-microduck: floor-care re-pinned to microduck (gripper-arm edge).
  const cafeMicro = readModel(SOURCES['cafe-stand']);
  for (const ref of cafeMicro.references) {
    if (ref.role === 'floor-care') {
      ref.project = 'microduck';
      ref.revision = hashes.microduck;
      ref.bounds = ['gripper-arm', 'gripper-camera', 'microduck'];
      ref.crossModelEdges = [{ from: 'req-floor-clear', relation: 'satisfiedBy', to: 'gripper-arm' }];
    } else {
      ref.revision = hashes[ref.project];
    }
  }
  const microHash = (await post(BASE + '/projects/cafe-stand/commits', {
    branch: 'with-microduck', author: 'admin', message: 'seed platform scenario', okf: cafeMicro,
  })).hash;
  console.log('cafe-stand with-microduck', microHash);
  console.log('SEED_OK');
}

main().catch((e) => { console.error('SEED_FAIL', e.message); process.exit(1); });
