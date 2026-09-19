// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Stops the server global-setup started and removes its temp directory.

const fs = require('node:fs');
const { stateFile } = require('./env');

module.exports = async function globalTeardown() {
  let state;
  try {
    state = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
  } catch {
    return; // nothing to clean up
  }

  if (state.pid) {
    try {
      process.kill(state.pid, 'SIGTERM');
    } catch {
      // already gone
    }
  }

  if (state.tmpDir) {
    try {
      fs.rmSync(state.tmpDir, { recursive: true, force: true });
    } catch {
      // ignore
    }
  }

  try {
    fs.rmSync(stateFile, { force: true });
  } catch {
    // ignore
  }
};
