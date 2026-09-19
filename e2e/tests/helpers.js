// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

const { baseURL } = require('../env');

// A CSS attribute selector safe for the opaque OKF ids. The corpus ids are
// alphanumeric-plus-underscore, but escaping is cheap insurance against a
// future id containing a quote or backslash.
function attrSelector(attr, value) {
  const escaped = String(value).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return `[${attr}="${escaped}"]`;
}

// The enhanced tree node for an element, keyed by the element's data-mw-id (or
// the synthetic activity:<n> id).
function nodeByKey(page, key) {
  return page.locator(`.mw-tree .mw-node${attrSelector('data-key', key)}`);
}

module.exports = { baseURL, attrSelector, nodeByKey };
