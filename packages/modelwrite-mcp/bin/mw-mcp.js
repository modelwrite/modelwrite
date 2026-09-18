#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The mw-mcp launcher. The real binary is platform-specific and is fetched by
// install.js (the postinstall script); this wrapper locates it, forwards every
// argument and the environment, and mirrors its exit status. It never downloads
// anything: if the binary is missing it fails loudly and says how to get it.

'use strict';

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const isWindows = process.platform === 'win32';
const localName = isWindows ? 'mw-mcp.exe' : 'mw-mcp';
const bin = path.join(__dirname, localName);

if (!fs.existsSync(bin)) {
  console.error(
    'modelwrite-mcp: the mw-mcp binary is missing at ' + bin + '.\n' +
    'It is fetched by the postinstall script when the package is installed.\n' +
    "Re-run 'npm rebuild modelwrite-mcp' (with network access) to fetch it,\n" +
    "or reinstall without --ignore-scripts."
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', (err) => {
  console.error('modelwrite-mcp: cannot run ' + bin + ': ' + err.message);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    // Mirror the signal so a parent sees the same termination reason.
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code === null ? 1 : code);
});
