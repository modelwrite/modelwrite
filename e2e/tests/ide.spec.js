// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Browser tests for the workbench's three-pane enhancement (server/src/ui/app.js).
// These exercise RUNTIME behaviour the Rust tests only assert structurally: the
// tree builds, clicking selects, the properties panel updates, search filters,
// keyboard navigation is wired, the diagram carries the selection hook, and deep
// links highlight the right entry. A JS file can be syntactically valid and do
// nothing; these tests prove app.js does something.

const { test, expect } = require('@playwright/test');
const { baseURL, attrSelector, nodeByKey } = require('./helpers');

const MODEL_URL = '/ui/projects/coffee/model';
const DIAGRAM_URL = '/ui/projects/coffee/diagram';

// The corpus counts, fixed by the committed coffee-machine fixture (the same
// numbers the Rust gate asserts: 25 requirements, 99 graph nodes, etc.).
const STRUCTURE_COUNT = 49;
const REQUIREMENT_COUNT = 25;
const SIGNAL_COUNT = 9;
const ACTIVITY_COUNT = 8;
const GRAPH_NODE_COUNT = 99;

async function idsOf(page, selector) {
  return page.$$eval(selector, (els) => els.map((el) => el.getAttribute('data-mw-id')));
}

// The properties panel as a list of [term, description] pairs, read from the
// single <dl> renderProps writes.
async function propsPairs(page) {
  return page.$eval('.mw-props-panel dl', (dl) => {
    const pairs = [];
    for (const node of dl.children) {
      if (node.tagName === 'DT') {
        pairs.push([node.textContent.trim(), null]);
      } else if (node.tagName === 'DD' && pairs.length) {
        pairs[pairs.length - 1][1] = node.textContent.trim();
      }
    }
    return pairs;
  });
}

function pairValue(pairs, term) {
  const found = pairs.find(([t]) => t === term);
  return found ? found[1] : undefined;
}

test.describe('model page', () => {
  test('the three-pane tree renders every structure element and requirement the page declares', async ({ page }) => {
    await page.goto(MODEL_URL);

    // The enhancement ran: the IDE shell and its three panes exist.
    await expect(page.locator('main.mw-model-ide')).toBeVisible();
    await expect(page.locator('.mw-tree-panel')).toHaveCount(1);
    await expect(page.locator('.mw-content')).toHaveCount(1);
    await expect(page.locator('.mw-props-panel')).toHaveCount(1);

    const structureIds = await idsOf(page, 'li.element[data-mw-id]');
    const requirementIds = await idsOf(page, 'tr.requirement[data-mw-id]');
    const signalIds = await idsOf(page, 'li.signal[data-mw-id]');
    const activityIds = await idsOf(page, 'div.activity[data-mw-id]');

    expect(structureIds).toHaveLength(STRUCTURE_COUNT);
    expect(requirementIds).toHaveLength(REQUIREMENT_COUNT);
    expect(signalIds).toHaveLength(SIGNAL_COUNT);
    expect(activityIds).toHaveLength(ACTIVITY_COUNT);

    const treeKeys = await page.$$eval('.mw-tree .mw-node', (els) =>
      els.map((el) => el.getAttribute('data-key'))
    );

    // Every element the page declares appears in the tree, by its exact id.
    for (const id of [...structureIds, ...requirementIds, ...signalIds]) {
      expect(treeKeys).toContain(id);
    }
    expect(treeKeys.filter((k) => k && k.startsWith('activity:'))).toHaveLength(ACTIVITY_COUNT);
    expect(treeKeys).toHaveLength(
      STRUCTURE_COUNT + REQUIREMENT_COUNT + SIGNAL_COUNT + ACTIVITY_COUNT
    );

    // Only non-empty groups render (the corpus has zero interfaces).
    const groupLabels = await page.$$eval('.mw-tree .mw-group-label', (els) =>
      els.map((el) => el.textContent.replace(/^[▾▸]/, ''))
    );
    expect(groupLabels).toEqual(['Structure', 'Requirements', 'Signals', 'Activities']);
  });

  test('clicking a tree node shows that element in the properties panel', async ({ page }) => {
    await page.goto(MODEL_URL);
    await expect(page.locator('.mw-workbench')).toHaveCount(1);

    // A structure element: name, id, kind.
    const struct = page.locator('li.element[data-mw-id]').first();
    const structId = await struct.getAttribute('data-mw-id');
    const structName = await struct.getAttribute('data-mw-name');
    const structKind = await struct.getAttribute('data-mw-kind');

    await nodeByKey(page, structId).click();
    await expect(page.locator('.mw-props-panel h3')).toHaveText(structName || structId);
    let pairs = await propsPairs(page);
    expect(pairValue(pairs, 'id')).toBe(structId);
    expect(pairValue(pairs, 'name')).toBe(structName);
    if (structKind) expect(pairValue(pairs, 'kind')).toBe(structKind);
    await expect(nodeByKey(page, structId)).toHaveClass(/mw-selected/);

    // A requirement: the requirement-only fields (reqId, coverage) appear, and
    // the coverage badge matches the page's declared data-mw-coverage.
    const req = page.locator('tr.requirement[data-mw-id]').first();
    const reqId = await req.getAttribute('data-mw-id');
    const reqName = await req.getAttribute('data-mw-name');
    const reqNum = await req.getAttribute('data-mw-reqid');
    const reqCoverage = await req.getAttribute('data-mw-coverage');

    await nodeByKey(page, reqId).click();
    await expect(page.locator('.mw-props-panel h3')).toHaveText(reqName || reqId);
    pairs = await propsPairs(page);
    expect(pairValue(pairs, 'id')).toBe(reqId);
    expect(pairValue(pairs, 'reqId')).toBe(reqNum);
    const badge = reqCoverage === 'covered' ? 'covered' : reqCoverage === 'uncovered' ? 'uncovered' : 'not reported';
    expect(pairValue(pairs, 'coverage')).toBe(badge);
  });

  test('the search box narrows the tree', async ({ page }) => {
    await page.goto(MODEL_URL);
    await expect(page.locator('.mw-workbench')).toHaveCount(1);

    const reqId = await page.locator('tr.requirement[data-mw-id]').first().getAttribute('data-mw-id');
    const structId = await page.locator('li.element[data-mw-id]').first().getAttribute('data-mw-id');

    const search = page.locator('.mw-search');
    await search.fill(reqId);

    // The matching node is visible, the non-matching one hidden, and the whole
    // tree narrows to exactly the one match.
    await expect(nodeByKey(page, reqId)).toBeVisible();
    await expect(nodeByKey(page, structId)).toBeHidden();
    expect(await page.locator('.mw-tree .mw-node:visible').count()).toBe(1);

    // Clearing restores the hidden node.
    await search.fill('');
    await expect(nodeByKey(page, structId)).toBeVisible();
  });

  test('?select=<id> deep link highlights the right entry', async ({ page }) => {
    await page.goto(MODEL_URL);
    const reqId = await page.locator('tr.requirement[data-mw-id]').first().getAttribute('data-mw-id');

    await page.goto(`${MODEL_URL}?select=${encodeURIComponent(reqId)}`);
    await expect(page.locator('.mw-workbench')).toHaveCount(1);
    await expect(nodeByKey(page, reqId)).toHaveClass(/mw-selected/);
    await expect(page.locator(attrSelector('data-mw-id', reqId))).toHaveClass(/mw-highlight/);
    await expect(page).toHaveURL(/select=/);
  });

  test('keyboard navigation selects nodes', async ({ page }) => {
    await page.goto(MODEL_URL);
    await expect(page.locator('.mw-workbench')).toHaveCount(1);

    // ArrowDown from nothing selects the first visible node.
    await page.keyboard.press('ArrowDown');
    const firstKey = await page.locator('.mw-tree .mw-node:visible').first().getAttribute('data-key');
    await expect(page.locator(`.mw-tree .mw-node${attrSelector('data-key', firstKey)}`)).toHaveClass(/mw-selected/);

    // ArrowUp keeps a selection within bounds.
    await page.keyboard.press('ArrowUp');
    await expect(page.locator(`.mw-tree .mw-node${attrSelector('data-key', firstKey)}`)).toHaveClass(/mw-selected/);
  });
});

test.describe('diagram page', () => {
  test('the diagram renders nodes carrying the hook and honours ?select=', async ({ page }) => {
    await page.goto(DIAGRAM_URL);
    await expect(page.locator('svg g.node[data-mw-id]').first()).toBeVisible();

    const diagramIds = await page.$$eval('svg g.node', (els) =>
      els.map((el) => el.getAttribute('data-mw-id'))
    );
    // Every node carries the hook the enhancement reads, and the diagram is
    // complete (one node per graph node in the corpus).
    expect(diagramIds.every((id) => id && id.length > 0)).toBe(true);
    expect(diagramIds).toHaveLength(GRAPH_NODE_COUNT);

    const firstId = diagramIds[0];
    await page.goto(`${DIAGRAM_URL}?select=${encodeURIComponent(firstId)}`);
    await expect(page.locator(`svg g.node${attrSelector('data-mw-id', firstId)}`)).toHaveClass(/mw-selected-node/);
  });
});

test.describe('no-JS contract', () => {
  test('with JavaScript disabled the page still shows every element', async ({ browser }) => {
    const context = await browser.newContext({ javaScriptEnabled: false });
    const page = await context.newPage();
    await page.goto(`${baseURL}${MODEL_URL}`);

    // The enhancement must not run.
    await expect(page.locator('.mw-workbench')).toHaveCount(0);

    // The server-rendered sections still carry every element (the Rust tests
    // assert this server-side; the browser assertion is a bonus, not a substitute).
    expect(await page.locator('li.element[data-mw-id]').count()).toBe(STRUCTURE_COUNT);
    expect(await page.locator('tr.requirement[data-mw-id]').count()).toBe(REQUIREMENT_COUNT);
    expect(await page.locator('li.signal[data-mw-id]').count()).toBe(SIGNAL_COUNT);
    expect(await page.locator('div.activity[data-mw-id]').count()).toBe(ACTIVITY_COUNT);

    await context.close();
  });
});
