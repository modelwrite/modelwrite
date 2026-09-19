// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

const path = require('node:path');

// The repository root is one level above this directory (e2e/).
const repoRoot = path.resolve(__dirname, '..');

// The port the test server binds. Defaults to 8099; override with MW_E2E_PORT.
// The server itself reads MW_PORT, but the test keeps its own port separate so a
// stray MW_PORT in the environment cannot move the server out from under the
// tests.
const port = Number(process.env.MW_E2E_PORT || 8099);

const baseURL = process.env.MW_E2E_BASE_URL || `http://127.0.0.1:${port}`;

// Where global-setup records the server pid and temp dir for global-teardown.
const stateFile = path.join(__dirname, '.state.json');

module.exports = { repoRoot, port, baseURL, stateFile };
