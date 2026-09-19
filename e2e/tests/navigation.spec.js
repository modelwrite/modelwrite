// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for the workbench navigation shell (N1/N2): the global top bar with the
// model switcher, the left navigator with its MODELS and SECTIONS groups, the context bar,
// and the per-section addresses. Everything under test is server-rendered links - the
// switcher is a plain <details> disclosure - so the navigator works with JavaScript disabled.

const { test, expect } = require('@playwright/test');
const { baseURL } = require('./helpers');

// The ten sections of a model, in navigator order. These are the SAME labels the server
// renders in server/src/ui/layout.rs::SECTIONS.
const SECTION_LABELS = [
  'Overview',
  'Structure',
  'Requirements',
  'Traceability',
  'Diagram',
  'Checks',
  'Changes',
  'Proposals',
  'Import',
  'Assist',
];

// A second model, so the MODELS group has more than one entry to switch between. Created
// once per suite over the throwaway database global-setup seeds.
test.describe('navigation shell', () => {
  test.beforeAll(async ({ request }) => {
    await request.post('/projects', { data: { name: 'tea' } });
  });

  test('the top bar and left navigator name the current model and mark it', async ({ page }) => {
    await page.goto('/ui/projects/coffee/overview');

    // The global switcher names the current model.
    await expect(page.locator('.site-header .switcher summary')).toHaveText('coffee');
    // The identity chip names who is viewing.
    await expect(page.locator('.site-header .chip')).toContainText('via');

    // MODELS group: both models, the current one highlighted.
    const modelLinks = page.locator('.rail .nav-group').nth(0).locator('a');
    await expect(modelLinks).toHaveCount(2);
    await expect(modelLinks.nth(0)).toHaveText('coffee');
    await expect(modelLinks.nth(1)).toHaveText('tea');
    await expect(modelLinks.nth(0)).toHaveClass(/current/);

    // SECTIONS group: all ten sections, Overview marked current.
    const sectionLinks = page.locator('.rail .nav-group').nth(1).locator('a');
    await expect(sectionLinks).toHaveCount(10);
    await expect(sectionLinks.nth(0)).toHaveText('Overview');
    await expect(sectionLinks.nth(0)).toHaveClass(/current/);
    expect(await sectionLinks.allTextContents()).toEqual(SECTION_LABELS);

    // The context bar shows the project and the version being viewed.
    await expect(page.locator('.context-bar .ctx-project')).toHaveText('coffee');
    await expect(page.locator('.context-bar')).toContainText('version');
  });

  test('switching model is one click and sections are reachable by address', async ({ page }) => {
    await page.goto('/ui/projects/coffee/overview');

    // One click in the MODELS group switches model.
    await page.locator('.rail .nav-group').nth(0).locator('a', { hasText: 'tea' }).click();
    await expect(page).toHaveURL(/\/ui\/projects\/tea\/overview/);
    await expect(page.locator('main h1')).toHaveText('Overview');

    // A section has its own address and renders its own content.
    await page.goto('/ui/projects/coffee/structure');
    await expect(page.locator('main h1')).toHaveText('Structure');
    await expect(page.locator('.structure-tree')).toHaveCount(1);
    await expect(
      page.locator('.rail .nav-group').nth(1).locator('a.current')
    ).toHaveText('Structure');
  });

  test('every section route renders its own heading', async ({ page }) => {
    const checks = [
      ['/ui/projects/coffee/overview', 'Overview'],
      ['/ui/projects/coffee/structure', 'Structure'],
      ['/ui/projects/coffee/requirements', 'Requirements'],
      ['/ui/projects/coffee/traceability', 'Traceability'],
      ['/ui/projects/coffee/diagram', 'Diagram'],
      ['/ui/projects/coffee/checks', 'Checks'],
      ['/ui/projects/coffee/changes', 'Changes'],
      ['/ui/projects/coffee/proposals', 'Proposals'],
      ['/ui/projects/coffee/import', 'Import'],
      ['/ui/projects/coffee/assist', 'Assist'],
    ];
    for (const [url, heading] of checks) {
      await page.goto(url);
      await expect(page.locator('main h1')).toHaveText(heading);
    }
  });
});

test.describe('navigation shell with JavaScript disabled', () => {
  test('the navigator is server-rendered links', async ({ browser }) => {
    const context = await browser.newContext({ javaScriptEnabled: false });
    const page = await context.newPage();
    await page.goto(baseURL + '/ui/projects/coffee/overview');

    // Both groups render as links, never as a JavaScript menu.
    const hrefs = await page.$$eval('.rail .nav-group a', (els) =>
      els.map((el) => el.getAttribute('href'))
    );
    expect(hrefs.length).toBeGreaterThanOrEqual(11); // models + 10 sections
    expect(hrefs.some((href) => href.startsWith('/ui/projects/coffee/structure'))).toBe(true);
    expect(
      hrefs.some((href) => href.startsWith('/ui/projects/coffee/requirements'))
    ).toBe(true);

    await context.close();
  });
});
