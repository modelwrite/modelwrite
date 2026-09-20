// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for N3: version control in the workbench. A version IS a branch, so these
// tests exercise the four actions over the existing branch/compare/merge endpoints: the
// version selector (switch version in one click, the choice in the URL), creating a new
// version, comparing the current version to another with both sides pre-filled, and making a
// version current with the gate verdict shown BEFORE the merge is offered.

const { test, expect } = require('@playwright/test');

async function post(request, url, data) {
  const response = await request.post(url, { data });
  if (!response.ok()) {
    throw new Error(
      response.status() + ' ' + response.statusText() + ': ' + (await response.text())
    );
  }
  return response;
}

test.describe('version control', () => {
  // A second version exists for the switch/compare/make-current tests. It is branched from
  // main's tip over the SAME JSON API the UI is a client of.
  test.beforeAll(async ({ request }) => {
    const branches = await (await request.get('/projects/coffee/branches')).json();
    const main = branches.find((b) => b.name === 'main');
    if (!branches.some((b) => b.name === 'v2')) {
      await post(request, '/projects/coffee/branches', { name: 'v2', from: main.tip });
    }
  });

  test('the context bar shows a version selector that switches in one click', async ({ page }) => {
    await page.goto('/ui/projects/coffee/overview');

    // The selector names the current version and, opened, lists every branch plus recent
    // commits as server-rendered links.
    const switcher = page.locator('.context-bar .version-switcher');
    await expect(switcher.locator('summary')).toContainText('main');
    await switcher.locator('summary').click();
    await expect(switcher.locator('.menu')).toBeVisible();
    await expect(switcher.locator('.menu a.branch', { hasText: 'v2' })).toBeVisible();
    await expect(switcher.locator('.menu li.vs-label')).toHaveText('Recent commits');

    // The current branch is marked, and switching is one click with the version in the URL.
    await expect(
      switcher.locator('a.branch[href="/ui/projects/coffee/overview?branch=main"]')
    ).toHaveClass(/current/);
    await switcher
      .locator('a.branch[href="/ui/projects/coffee/overview?branch=v2"]')
      .click();
    await expect(page).toHaveURL(/\/ui\/projects\/coffee\/overview\?branch=v2/);
    await expect(switcher.locator('summary')).toContainText('v2');
  });

  test('a new version is named, branched, and lands on the new branch', async ({ page }) => {
    await page.goto('/ui/projects/coffee/version/new');
    await expect(page.locator('main h1')).toHaveText('New version');

    // The choices are the branch tips and the recent commits (deduplicated).
    const options = page.locator('#version-from option');
    await expect(options.nth(0)).toHaveText(/main @ [0-9a-f]{8}/);

    await page.fill('#version-name', 'v3');
    await page.click('button[type="submit"]');

    // Landed on the new version, with the version in the address.
    await expect(page).toHaveURL(/\/ui\/projects\/coffee\/overview\?branch=v3/);
    await expect(page.locator('main h1')).toHaveText('Overview');
    await expect(page.locator('.context-bar .version-switcher summary')).toContainText('v3');
  });

  test('compare is one click from the selector with both sides pre-filled', async ({ page }) => {
    await page.goto('/ui/projects/coffee/overview?branch=main');

    const switcher = page.locator('.context-bar .version-switcher');
    await switcher.locator('summary').click();
    await switcher
      .locator('a.compare[href="/ui/projects/coffee/compare?from=main&to=v2"]')
      .click();

    await expect(page).toHaveURL(/\/ui\/projects\/coffee\/compare\?from=main&to=v2/);
    await expect(page.locator('main h1')).toHaveText('Compare');
    // The engine's section-scoped diff AND its gate verdict are both rendered.
    await expect(page.locator('#diff')).toBeVisible();
    await expect(page.locator('#gate')).toContainText('Verdict');
  });

  test('make current shows the gate verdict before the merge form', async ({ page }) => {
    await page.goto('/ui/projects/coffee/version/make-current?candidate=v2');
    await expect(page.locator('main h1')).toHaveText('Make current');

    // The gate section precedes the merge section, so the engineer decides with the evidence
    // in front of them.
    const order = await page.$$eval('main .model-section', (els) =>
      els.map((el) => el.id)
    );
    expect(order.indexOf('gate')).toBeGreaterThanOrEqual(0);
    expect(order.indexOf('merge')).toBeGreaterThanOrEqual(0);
    expect(order.indexOf('gate')).toBeLessThan(order.indexOf('merge'));

    // The gate carries the engine's verdict, never the page's own opinion.
    await expect(page.locator('#gate')).toContainText('Verdict');
    const verdict = page.locator('#gate .covered, #gate .uncovered').first();
    await expect(verdict).toHaveText(/passed|failed/);

    // The merge form is the existing one, targeting main with the candidate fixed.
    await expect(page.locator('#merge input[name="branch"]')).toHaveValue('main');
    await expect(page.locator('#merge input[name="other"]')).toHaveValue('v2');
  });
});
