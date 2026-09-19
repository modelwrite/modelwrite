// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Server-rendered requirements table: an allocation's opaque element id wraps
// inside its column (never bleeds into the next one), the full id stays on
// hover via a title, and the coverage chip is labelled as the requirement-level
// verdict rather than a per-link state.

const { test, expect } = require('@playwright/test');

test.describe('requirements table', () => {
  test('an allocation id wraps inside its column and exposes the full id on hover', async ({ page }) => {
    await page.goto('/ui/projects/coffee/requirements');
    await expect(page.locator('main.mw-model-ide')).toBeVisible();

    // No satisfied-by cell lets its content run past the column boundary.
    const overflows = await page.$$eval('table.requirements td.req-satisfied', (cells) =>
      cells
        .filter((td) => td.scrollWidth > td.clientWidth + 1)
        .map((td) => td.textContent.trim().slice(0, 40))
    );
    expect(overflows).toEqual([]);

    // A broken link carries the per-link marker and the full target id on hover.
    const broken = page.locator('table.requirements td.req-satisfied span.broken').first();
    await expect(broken).toContainText('unresolved link');
    await expect(broken).toHaveAttribute('title', /^_2026x_/);

    // The coverage chip is the requirement's verdict, stated on hover.
    const chip = page.locator('table.requirements td.req-coverage .covered').first();
    await expect(chip).toHaveAttribute('title', /Requirement coverage/);
  });
});
