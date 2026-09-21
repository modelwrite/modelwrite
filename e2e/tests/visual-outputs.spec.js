// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// V1 visual outputs: the SVG download and the full-screen presentation view. These are browser
// tests, and the presentation ones run with JavaScript disabled, because both outputs must be
// plain server-rendered SVG and HTML: a visitor must be able to take a diagram out of the page
// with no script at all.

const { test, expect } = require('@playwright/test');

const DIAGRAM = '/ui/projects/coffee/diagram';
const DOWNLOAD = '/ui/projects/coffee/diagram.svg';

test.describe('visual outputs with JavaScript disabled', () => {
  test.use({ javaScriptEnabled: false });

  test('the diagram page offers both outputs and the presentation view is chrome-free', async ({ page }) => {
    await page.goto(DIAGRAM);

    const present = page.getByRole('link', { name: 'Present', exact: true });
    const download = page.getByRole('link', { name: 'Download SVG', exact: true });
    await expect(present).toBeVisible();
    await expect(download).toBeVisible();

    await present.click();
    await expect(page).toHaveURL(/\/ui\/projects\/coffee\/present\?commit=[0-9a-f]+&view=structure/);

    // The diagram is there, at viewport scale, and the workbench chrome is gone.
    await expect(page.locator('svg.mw-diagram-svg')).toBeVisible();
    await expect(page.locator('header.site-header')).toHaveCount(0);
    await expect(page.locator('nav.rail')).toHaveCount(0);
    await expect(page.locator('.context-bar')).toHaveCount(0);
    await expect(page.locator('script')).toHaveCount(0);

    // The project and revision stay on the page, and the download is one click away at the same
    // revision.
    await expect(page.locator('body')).toContainText('coffee');
    await expect(page.locator('body')).toContainText('commit');
    await expect(page.locator('a.download')).toHaveAttribute(
      'href',
      /\/ui\/projects\/coffee\/diagram\.svg\?commit=[0-9a-f]+&view=structure/
    );
  });

  test('the presentation view fetches nothing external', async ({ page }) => {
    const external = [];
    page.on('request', (request) => {
      const host = new URL(request.url()).hostname;
      if (host !== '127.0.0.1' && host !== 'localhost') {
        external.push(request.url());
      }
    });
    await page.goto('/ui/projects/coffee/present?branch=main&view=structure');
    await page.waitForLoadState('load');
    await expect(page.locator('svg.mw-diagram-svg')).toBeVisible();
    expect(external).toEqual([]);
  });
});

test('the SVG export is byte-identical across two downloads and states its provenance', async ({ request }) => {
  const page = await request.get(DIAGRAM);
  const html = await page.text();
  const match = html.match(/\/ui\/projects\/coffee\/diagram\.svg\?commit=([0-9a-f]+)&amp;view=structure/);
  expect(match).not.toBeNull();
  const hash = match[1];

  const url = DOWNLOAD + '?commit=' + hash + '&view=structure';
  const first = await request.get(url);
  const second = await request.get(url);
  expect(first.status()).toBe(200);
  expect(second.status()).toBe(200);
  expect(first.headers()['content-type']).toContain('image/svg+xml');
  expect(first.headers()['content-disposition']).toContain(hash);

  const a = await first.body();
  const b = await second.body();
  // THE PROOF: two exports of the same revision are byte-identical.
  expect(a.equals(b)).toBe(true);

  const svg = a.toString('utf8');
  expect(svg.startsWith('<?xml')).toBe(true);
  expect(svg).toContain('modelwrite-provenance:');
  expect(svg).toContain('project=coffee');
  expect(svg).toContain('commit=' + hash);
  expect(svg).toContain('renderer=mw-diagram-svg/');
  expect(svg).toContain('xmlns:mw=');
  expect(svg).toContain('rendererVersion=');
  expect(svg).toContain('<style>');
  expect(svg).toContain('viewBox=');

  // Nothing in the file can trigger a network request.
  expect(svg).not.toMatch(/<(script|image|foreignObject)\b/);
  expect(svg).not.toContain('href');
  expect(svg).not.toContain('xlink:');
  expect(svg).not.toContain('url(');
  expect(svg).not.toContain('@import');
});
