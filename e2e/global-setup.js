// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

// Starts mw-server against a throwaway SQLite database, seeds it, and leaves it
// running for the tests. global-teardown reads the pid and temp dir back out of
// the state file and shuts the server down.

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { repoRoot, port, baseURL, stateFile } = require('./env');
const { seed } = require('./seed');

function serverBinary() {
  if (process.env.MW_E2E_SERVER_BIN) {
    return process.env.MW_E2E_SERVER_BIN;
  }
  const name = process.platform === 'win32' ? 'mw-server.exe' : 'mw-server';
  return path.join(repoRoot, 'target', 'debug', name);
}

async function waitForHealth(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let last = 'no attempt yet';
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${url}/health`);
      if (response.ok) return;
      last = `health returned ${response.status}`;
    } catch (error) {
      last = error.message;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`server at ${url} did not become healthy in ${timeoutMs}ms: ${last}`);
}

module.exports = async function globalSetup() {
  const binary = serverBinary();
  if (!fs.existsSync(binary)) {
    throw new Error(`${binary} not found. Build it first: cargo build -p mw-server`);
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mw-e2e-'));
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      MW_DB: path.join(tmpDir, 'mw.db'),
      MW_PORT: String(port),
      MW_EVIDENCE_DIR: path.join(tmpDir, 'evidence'),
      MW_BIND: '127.0.0.1',
    },
    stdio: 'inherit',
  });
  // The server outlives this setup process; unref so nothing here waits on it.
  child.unref();

  try {
    await waitForHealth(baseURL, 30000);
    const { project, hash } = await seed(baseURL);
    fs.writeFileSync(
      stateFile,
      JSON.stringify({ pid: child.pid, tmpDir, baseURL, project, commitHash: hash }, null, 2)
    );
  } catch (error) {
    try { child.kill(); } catch { /* ignore */ }
    throw error;
  }
};
