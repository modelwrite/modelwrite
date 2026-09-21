// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// E1 proof: the drop-zone front door completes with JavaScript DISABLED.
//
// The drop zone is progressive enhancement. The inline script only adds
// drag-and-drop and a file-name echo on top of an ordinary multipart file input,
// and the result page is server-rendered end to end. This spec drives the whole
// flow - drop the real MagicDraw export, read the plain-language summary the
// engine produced, press the ONE button, land on the health view - in a browser
// context where no script ever runs.

const path = require('node:path');
const { test, expect } = require('@playwright/test');
const { repoRoot } = require('../env');

test.use({ javaScriptEnabled: false });

const MODEL = path.join(repoRoot, 'real-world', 'mdzip', 'model.xmi');

test('the drop zone completes the import with JavaScript disabled', async ({ page }) => {
  await page.goto('/ui');

  // The front door is the drop zone, not a list and a New project box.
  const zone = page.locator('#mw-dropzone');
  await expect(zone).toBeVisible();
  await expect(zone.locator('h1')).toHaveText(
    'Drop your SysML model here, or choose a file'
  );

  // No drag-and-drop, no script: the file input and the submit button ARE the flow.
  await zone.locator('input[type="file"]').setInputFiles(MODEL);
  await zone.getByRole('button', { name: 'Onboard this model' }).click();

  // The result is server-rendered, and it speaks the engine's own numbers.
  await expect(page.locator('#summary')).toBeVisible();
  const summary = await page.locator('#summary p').allTextContents();
  const prose = summary.join('\n');
  console.log('PLAIN-LANGUAGE SUMMARY (JavaScript disabled):\n' + prose);
  expect(summary.length).toBeGreaterThan(3);
  expect(prose).toContain(
    'It carried 41 blocks, 95 relationships and 25 requirements.'
  );
  expect(prose).toContain('252 things are outside what this reader carries');
  expect(prose).toContain('The five biggest classes of loss are');
  await expect(page.locator('.onboard-facts')).toBeVisible();

  // ONE button, exactly as the design states it, and it commits the import.
  const one = page.getByRole('button', {
    name: 'Import 41 blocks and 95 relationships, accepting these 252 named losses',
  });
  await expect(one).toBeVisible();
  await one.click();

  await expect(page.locator('h1')).toContainText('Onboarded: model.xmi');
  await expect(page.locator('#summary')).toContainText('It round-trips exactly');
  await expect(page.locator('#summary')).toContainText(
    'This is a SysML v1 XMI export'
  );

  // The health view is one click from the result.
  const health = page.getByRole('link', { name: 'Open the model-health view' });
  await expect(health).toBeVisible();
  await health.click();
  await expect(page.locator('h1')).toHaveText('Model health');
});
