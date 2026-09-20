// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for the compare page's VARIANT IMPACT panel. When a platform model has two
// versions that choose different subsystems, the panel lists both sides' references (role
// keyed, a changed role marked), renders each platform requirement's coverage state computed
// by the ENGINE (never a reimplementation), and states the honest measured-vs-asserted
// boundary: what is computed on each side versus what is not proved inside a subsystem.

const fs = require('node:fs');
const path = require('node:path');
const { test, expect } = require('@playwright/test');

const repoRoot = path.resolve(__dirname, '..', '..');
const modelsDir = path.join(__dirname, '..', 'models');
const corpusPath = path.join(
  repoRoot,
  'sample', 'corpus', 'coffee-machine', 'okf', 'expected',
  'coffee_machine_model.json'
);

function readModel(name) {
  return JSON.parse(fs.readFileSync(path.join(modelsDir, name), 'utf8'));
}

async function post(request, url, data) {
  const response = await request.post(url, { data });
  if (!response.ok()) {
    throw new Error(response.status() + ' ' + response.statusText() + ': ' + (await response.text()));
  }
  return response;
}

async function ensureProject(request, name) {
  const response = await request.post('/projects', { data: { name } });
  // A 409 means another spec (composition) already created it; that is fine.
  if (!response.ok() && response.status() !== 409) {
    throw new Error(response.status() + ' ' + response.statusText() + ': ' + (await response.text()));
  }
}

async function commit(request, project, okf, branch) {
  const response = await post(request, '/projects/' + project + '/commits', {
    branch,
    author: 'e2e',
    message: 'seed variant impact scenario',
    okf,
  });
  return (await response.json()).hash;
}

test.describe('variant impact panel', () => {
  // Seed the four subsystems plus microduck, then two versions of the cafe-stand platform:
  // main pins floor-care to floor-robot, with-microduck re-pins it to microduck.
  test.beforeAll(async ({ request }) => {
    for (const name of [
      'purchasing-terminal',
      'coffee-machine',
      'sandwich-toaster',
      'floor-robot',
      'microduck',
      'cafe-stand',
    ]) {
      await ensureProject(request, name);
    }

    const hashes = {
      'purchasing-terminal': await commit(request, 'purchasing-terminal', readModel('purchasing-terminal.json'), 'main'),
      'coffee-machine': await commit(request, 'coffee-machine', JSON.parse(fs.readFileSync(corpusPath, 'utf8')), 'main'),
      'sandwich-toaster': await commit(request, 'sandwich-toaster', readModel('sandwich-toaster.json'), 'main'),
      'floor-robot': await commit(request, 'floor-robot', readModel('floor-robot.json'), 'main'),
      microduck: await commit(request, 'microduck', readModel('microduck.json'), 'main'),
    };

    // main: the platform as the fixture declares it, floor-care -> floor-robot.
    const cafeStand = readModel('cafe-stand.json');
    for (const reference of cafeStand.references) {
      reference.revision = hashes[reference.project];
    }
    await commit(request, 'cafe-stand', cafeStand, 'main');

    // with-microduck: the same platform with floor-care re-pinned to microduck.
    const cafeMicro = readModel('cafe-stand.json');
    for (const reference of cafeMicro.references) {
      if (reference.role === 'floor-care') {
        reference.project = 'microduck';
        reference.revision = hashes.microduck;
        reference.bounds = ['gripper-arm', 'gripper-camera', 'microduck'];
      } else {
        reference.revision = hashes[reference.project];
      }
    }
    await commit(request, 'cafe-stand', cafeMicro, 'with-microduck');
  });

  test('lists both sides references and marks the changed role', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/compare?from=main&to=with-microduck');
    await expect(page.locator('main h1')).toHaveText('Compare');

    const section = page.locator('#impact');
    await expect(section).toBeVisible();

    // Four roles on each side, shown role -> project@revision.
    const rows = section.locator('tr.impact-ref');
    await expect(rows).toHaveCount(4);

    const payment = section.locator('tr.impact-ref[data-role="payment"]');
    await expect(payment.locator('td.impact-side').nth(0)).toContainText('purchasing-terminal');
    await expect(payment.locator('td.impact-side').nth(1)).toContainText('purchasing-terminal');
    await expect(payment).toHaveAttribute('data-changed', 'false');

    // floor-care is the changed choice: floor-robot on main, microduck on with-microduck.
    const floor = section.locator('tr.impact-ref[data-role="floor-care"]');
    await expect(floor.locator('td.impact-side').nth(0)).toContainText('floor-robot');
    await expect(floor.locator('td.impact-side').nth(1)).toContainText('microduck');
    await expect(floor).toHaveAttribute('data-changed', 'true');
    await expect(floor.locator('.impact-changed')).toHaveText('changed');
  });

  test('renders each requirement coverage state from the engine', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/compare?from=main&to=with-microduck');

    const reqs = page.locator('#impact li.impact-req');
    await expect(reqs).toHaveCount(4);

    // Both versions cover all four platform requirements on their own graph, so the engine's
    // coverage renders every requirement as "both".
    for (const reqId of ['CS-1', 'CS-2', 'CS-3', 'CS-4']) {
      const row = page.locator('#impact li.impact-req[data-req-id="' + reqId + '"]');
      await expect(row).toHaveAttribute('data-state', 'both');
      await expect(row.locator('.impact-state')).toHaveText('both');
    }

    await expect(page.locator('#impact .impact-summary')).toContainText('satisfied in both versions');
  });

  test('states the measured-vs-asserted boundary', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/compare?from=main&to=with-microduck');

    const boundary = page.locator('#impact .impact-boundary');
    await expect(boundary).toBeVisible();

    // Computed: the platform's own coverage on each side, and the reference differences.
    await expect(boundary.locator('.boundary-measured')).toContainText('requirement coverage on each side');
    await expect(boundary.locator('.boundary-measured')).toContainText('reference differences');

    // Not computed: coverage proved inside a referenced subsystem, explained-but-not-proved.
    await expect(boundary.locator('.boundary-asserted')).toContainText('referenced subsystem');
    await expect(boundary.locator('.boundary-asserted')).toContainText('explained by the change but not proved');

    // The allocatedTo link is a role name, not yet a first-class cross-model edge.
    await expect(boundary.locator('.fidelity-note')).toContainText('allocatedTo');
    await expect(boundary.locator('.fidelity-note')).toContainText('not a first-class cross-model edge');
  });
});
