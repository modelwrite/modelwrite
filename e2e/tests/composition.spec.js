// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for the Composition section: the system-of-systems view (each subsystem
// reference with its role, pinned revision and resolve state) and the business-process view
// (the ordered flow across subsystems, each step carrying its allocated-to chip and the
// requirement it satisfies), plus the measured-vs-asserted boundary and the allocatedTo
// limitation. The cafe-stand platform scenario is seeded in beforeAll over the SAME JSON API
// the Rust references/composition tests use, with the references rewritten to the actual
// committed hashes so every reference resolves.

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
    throw new Error(`${response.status()} ${response.statusText()}: ${await response.text()}`);
  }
  return response;
}

async function commit(request, project, okf) {
  const response = await post(request, `/projects/${project}/commits`, {
    branch: 'main',
    author: 'e2e',
    message: 'seed platform scenario',
    okf,
  });
  return (await response.json()).hash;
}

// Seed the four subsystems, then cafe-stand as the platform whose references point at the
// actual subsystem commit hashes (the hashes are content addresses over project+branch+okf,
// so they are rewritten after seeding rather than taken from the fixture).
async function seedPlatform(request) {
  for (const name of ['purchasing-terminal', 'coffee-machine', 'sandwich-toaster', 'floor-robot', 'cafe-stand']) {
    await post(request, '/projects', { name });
  }

  const hashes = {
    'purchasing-terminal': await commit(
      request, 'purchasing-terminal', readModel('purchasing-terminal.json')
    ),
    'coffee-machine': await commit(
      request, 'coffee-machine', JSON.parse(fs.readFileSync(corpusPath, 'utf8'))
    ),
    'sandwich-toaster': await commit(
      request, 'sandwich-toaster', readModel('sandwich-toaster.json')
    ),
    'floor-robot': await commit(
      request, 'floor-robot', readModel('floor-robot.json')
    ),
  };

  const cafeStand = readModel('cafe-stand.json');
  for (const reference of cafeStand.references) {
    reference.revision = hashes[reference.project];
  }
  const cafeHash = await commit(request, 'cafe-stand', cafeStand);
  return { hashes, cafeHash };
}

test.describe('composition section', () => {
  test.beforeAll(async ({ request }) => {
    await seedPlatform(request);
  });

  test('lists each subsystem with role, pinned revision and resolve state', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/composition');
    await expect(page.locator('main h1')).toHaveText('Composition');

    await expect(page.locator('.composition-summary')).toHaveText(
      '3 subsystems, all resolving at their pinned revisions'
    );

    const cards = page.locator('li.subsystem-card');
    await expect(cards).toHaveCount(3);

    const expected = [
      { role: 'payment', project: 'purchasing-terminal' },
      { role: 'beverage', project: 'coffee-machine' },
      { role: 'food', project: 'sandwich-toaster' },
    ];
    for (let index = 0; index < expected.length; index += 1) {
      const card = cards.nth(index);
      await expect(card.locator('.role-chip')).toHaveText(expected[index].role);
      await expect(card.locator('.subsystem-project')).toHaveText(expected[index].project);
      await expect(card.locator('.subsystem-revision')).toContainText('revision');
      // Short hash shown, full hash on hover via the title attribute.
      await expect(card.locator('.subsystem-revision code')).toHaveAttribute('title', /^[0-9a-f]{64}$/);
      await expect(card.locator('.subsystem-revision code')).toHaveText(/^[0-9a-f]{8}$/);
      // The resolve chip is green (resolves), carried by both a class and a data attribute
      // so the state is never colour alone.
      await expect(card).toHaveAttribute('data-resolves', 'true');
      await expect(card.locator('.covered')).toHaveText('resolves');
    }
  });

  test('renders the process steps in order with their allocated-to chips', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/composition');

    const steps = page.locator('.flow-step');
    await expect(steps).toHaveCount(5);

    // Ordered left-to-right in the DOM: Take Order -> Take Payment -> Provision Beverage and
    // Provision Food (a parallel branch) -> Complete Order.
    const names = await steps.locator('.step-name').allTextContents();
    expect(names).toEqual([
      'Take Order',
      'Take Payment',
      'Provision Beverage',
      'Provision Food',
      'Complete Order',
    ]);

    // Each step shows the subsystem it is allocatedTo as a chip.
    const roles = await steps.locator('.role-chip').allTextContents();
    expect(roles).toEqual([
      'cafe-stand',
      'purchasing-terminal',
      'coffee-machine',
      'sandwich-toaster',
      'cafe-stand',
    ]);

    // The two provisioning steps are one parallel layer, not two sequential ones.
    await expect(page.locator('.flow-parallel-label')).toHaveText('parallel');

    // Each step shows the requirement it satisfies where a Satisfy edge exists.
    const takePayment = page.locator('.flow-step').filter({ hasText: 'Take Payment' });
    await expect(takePayment.locator('.step-satisfies')).toContainText('CS-2');
    await expect(takePayment.locator('.step-satisfies')).toContainText('One Payment');

    const completeSatisfies = await page
      .locator('.flow-step')
      .filter({ hasText: 'Complete Order' })
      .locator('.step-satisfies')
      .allTextContents();
    expect(completeSatisfies.some((s) => s.includes('CS-1'))).toBe(true);
    expect(completeSatisfies.some((s) => s.includes('CS-3'))).toBe(true);
  });

  test('states the measured-vs-asserted boundary and the allocatedTo limitation', async ({ page }) => {
    await page.goto('/ui/projects/cafe-stand/composition');

    // What the platform proves, and only that, is the "measured" list.
    const measured = page.locator('.boundary-measured');
    await expect(measured).toContainText('every subsystem reference resolves at its pinned revision');
    await expect(measured).toContainText('every integrated revision was itself gated');
    await expect(measured).toContainText('every platform requirement is covered within the platform model');

    // The global graph property is asserted, not proved, and stays in its own panel.
    await expect(page.locator('.boundary-asserted')).toContainText('global graph property');

    // The allocatedTo link is a role name, not yet a first-class cross-model edge.
    const fidelity = page.locator('.fidelity-note');
    await expect(fidelity).toContainText('allocatedTo');
    await expect(fidelity).toContainText('not yet a first-class cross-model edge');
  });

  test('a model without references says so and points at the platform example', async ({ page }) => {
    await page.goto('/ui/projects/coffee/composition');
    await expect(page.locator('main h1')).toHaveText('Composition');

    const section = page.locator('#subsystems');
    await expect(section).toContainText('no subsystem references');
    await expect(section.locator('a')).toHaveAttribute('href', '/ui/projects/cafe-stand/composition');
    await expect(section.locator('a')).toHaveText('cafe-stand');
  });
});
