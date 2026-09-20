// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for the diagram presentation: the structure view is complete and deterministic,
// the kind filter dims nodes of other kinds, and the process view lays the cafe-stand activity
// flow left-to-right with its parallel branch side by side.

const fs = require('node:fs');
const path = require('node:path');
const { test, expect } = require('@playwright/test');
const { attrSelector } = require('./helpers');

const repoRoot = path.resolve(__dirname, '..', '..');
const modelsDir = path.join(__dirname, '..', 'models');
const DIAGRAM_URL = '/ui/projects/coffee/diagram';

function readModel(name) {
  return JSON.parse(fs.readFileSync(path.join(modelsDir, name), 'utf8'));
}

async function post(request, url, data) {
  const response = await request.post(url, { data });
  if (!response.ok()) {
    throw new Error(response.status() + ' ' + response.statusText() + ': ' + await response.text());
  }
  return response;
}

async function commit(request, project, okf) {
  const response = await post(request, '/projects/' + project + '/commits', {
    branch: 'main',
    author: 'e2e',
    message: 'seed diagram scenario',
    okf,
  });
  return (await response.json()).hash;
}

// Create a project, tolerating the case where another spec (composition) already seeded it. The
// shared server runs every spec file against one database, so seeding must be idempotent.
async function ensureProject(request, name) {
  const response = await request.post('/projects', { data: { name } });
  if (response.status() === 409) {
    return;
  }
  if (!response.ok()) {
    throw new Error(response.status() + ' ' + response.statusText() + ': ' + await response.text());
  }
}

// Seed the platform so cafe-stand has a process flow to draw, with its references rewritten to
// the actual subsystem hashes (the same scenario the composition spec uses).
async function seedPlatform(request) {
  for (const name of ['purchasing-terminal', 'coffee-machine', 'sandwich-toaster', 'cafe-stand']) {
    await ensureProject(request, name);
  }
  const hashes = {
    'purchasing-terminal': await commit(request, 'purchasing-terminal', readModel('purchasing-terminal.json')),
    'coffee-machine': await commit(
      request,
      'coffee-machine',
      JSON.parse(fs.readFileSync(path.join(repoRoot, 'sample', 'corpus', 'coffee-machine', 'okf', 'expected', 'coffee_machine_model.json'), 'utf8'))
    ),
    'sandwich-toaster': await commit(request, 'sandwich-toaster', readModel('sandwich-toaster.json')),
  };
  const cafeStand = readModel('cafe-stand.json');
  for (const reference of cafeStand.references) {
    reference.revision = hashes[reference.project];
  }
  await commit(request, 'cafe-stand', cafeStand);
}

// The placed box geometry of every node, as a stable string, for the determinism assertion.
async function nodeGeometry(page) {
  return page.$$eval('svg g.node', (els) =>
    els
      .map((el) => {
        const rect = el.querySelector('rect');
        const x = rect ? rect.getAttribute('x') : '';
        const y = rect ? rect.getAttribute('y') : '';
        return el.getAttribute('data-mw-id') + '@' + x + ',' + y;
      })
      .sort()
  );
}

function rectX(page, id) {
  return page.getAttribute('svg g.node' + attrSelector('data-mw-id', id) + ' rect', 'x');
}

test.describe('structure diagram', () => {
  test('renders every node deterministically across loads', async ({ page }) => {
    await page.goto(DIAGRAM_URL);
    await expect(page.locator('svg g.node[data-mw-id]').first()).toBeVisible();

    const first = await nodeGeometry(page);
    // Completeness: one box per corpus graph node.
    expect(first).toHaveLength(99);

    await page.reload();
    await expect(page.locator('svg g.node[data-mw-id]').first()).toBeVisible();
    const second = await nodeGeometry(page);
    // Determinism: the same model lays out byte-identically every load.
    expect(second).toEqual(first);
  });

  test('the kind filter dims nodes of every other kind', async ({ page }) => {
    await page.goto(DIAGRAM_URL);
    await expect(page.locator('svg g.node[data-mw-id]').first()).toBeVisible();

    const chip = page.locator('.kind-filter[data-mw-kind="requirement"]');
    await expect(chip).toBeVisible();
    await chip.click();
    await expect(chip).toHaveClass(/active/);

    // Requirements stay visible; blocks are filtered out.
    await expect(page.locator('svg g.node[data-mw-kind="requirement"]').first()).not.toHaveClass(/mw-filtered-out/);
    await expect(page.locator('svg g.node[data-mw-kind="block"]').first()).toHaveClass(/mw-filtered-out/);

    // Clearing the filter restores the block.
    await chip.click();
    await expect(page.locator('svg g.node[data-mw-kind="block"]').first()).not.toHaveClass(/mw-filtered-out/);
  });

  test('a model without an activity flow offers no process view', async ({ page }) => {
    await page.goto(DIAGRAM_URL);
    await expect(page.locator('.view-toggle .view-option').first()).toHaveText('Structure');
    await expect(page.locator('.view-toggle .view-option', { hasText: 'Process' })).toHaveCount(0);
  });
});

test.describe('process diagram', () => {
  test.beforeAll(async ({ request }) => {
    await seedPlatform(request);
  });

  test('lays the cafe-stand flow left-to-right with a parallel branch', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/diagram?view=process');
    await expect(page.locator('.view-toggle .view-option.current')).toHaveText('Process');

    const xOrder = async (id) => Number(await rectX(page, id));
    const order = await xOrder('act-order');
    const pay = await xOrder('act-pay');
    const brew = await xOrder('act-brew');
    const toast = await xOrder('act-toast');
    const complete = await xOrder('act-complete');

    expect(order).toBeLessThan(pay);
    expect(pay).toBeLessThan(brew);
    expect(pay).toBeLessThan(toast);
    expect(brew).toBeLessThan(complete);
    expect(toast).toBeLessThan(complete);
    // The two provisioning steps are one parallel layer, side by side.
    expect(brew).toBe(toast);
  });
});
