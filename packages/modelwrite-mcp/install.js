#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// modelwrite-mcp postinstall: fetch the prebuilt mw-mcp binary for this platform.
//
// The binary is built and attached to a GitHub Release by the release workflow.
// This script downloads exactly that binary, for exactly this platform, and
// refuses to leave a half-installed package behind: any failure exits non-zero
// so 'npm install' fails rather than silently installing something unusable.

'use strict';

const fs = require('fs');
const path = require('path');
const http = require('http');
const https = require('https');

const pkg = require('./package.json');
const VERSION = pkg.version;

// Where the binary lives. Override for a mirror or an air-gapped registry.
const DEFAULT_BASE = 'https://github.com/modelwrite/modelwrite/releases/download';
const base = (process.env.MW_MCP_DOWNLOAD_BASE || DEFAULT_BASE).replace(/\/+$/, '');

// The one place that maps process.platform + process.arch to the release asset
// the release workflow uploads. Unsupported platforms fail loudly, never fall
// back to a wrong-architecture binary.
const ASSETS = {
  'linux-x64': 'mw-mcp-linux-x64',
  'linux-arm64': 'mw-mcp-linux-arm64',
  'darwin-x64': 'mw-mcp-darwin-x64',
  'darwin-arm64': 'mw-mcp-darwin-arm64',
  'win32-x64': 'mw-mcp-windows-x64.exe',
};

function fail(message) {
  console.error('modelwrite-mcp: ' + message);
  process.exit(1);
}

const key = process.platform + '-' + process.arch;
const asset = ASSETS[key];
if (!asset) {
  fail(
    'no prebuilt mw-mcp binary for ' + process.platform + '/' + process.arch +
    '. Supported platforms: linux-x64, linux-arm64, darwin-x64, darwin-arm64, win32-x64.'
  );
}

const isWindows = process.platform === 'win32';
const localName = isWindows ? 'mw-mcp.exe' : 'mw-mcp';
const dest = path.join(__dirname, 'bin', localName);
const url = base + '/v' + VERSION + '/' + asset;

// A reinstall (or a partial retry) must not re-download: if the binary is
// already in place it is trusted as-is.
if (fs.existsSync(dest)) {
  if (!isWindows) {
    fs.chmodSync(dest, 0o755);
  }
  console.log('modelwrite-mcp: ' + localName + ' already present; skipping download');
  process.exit(0);
}

fs.mkdirSync(path.dirname(dest), { recursive: true });

download(url, dest)
  .then(() => {
    // A zero-byte or missing binary is not usable: refuse to leave it behind.
    const size = fs.existsSync(dest) ? fs.statSync(dest).size : 0;
    if (size === 0) {
      fail('downloaded an empty binary from ' + url);
    }
    if (!isWindows) {
      fs.chmodSync(dest, 0o755);
    }
    console.log('modelwrite-mcp: fetched ' + asset);
  })
  .catch((err) => {
    // Never leave a partial download where the wrapper would find it.
    try { fs.unlinkSync(dest); } catch (_) { /* nothing to remove */ }
    fail(
      'could not fetch the mw-mcp binary from ' + url + ': ' + err.message +
      '. A release must be published first (see .github/workflows/release.yml), ' +
      'or set MW_MCP_DOWNLOAD_BASE to a mirror that hosts this asset.'
    );
  });

function download(url, dest, redirects) {
  redirects = redirects || 0;
  return new Promise((resolve, reject) => {
    const lib = url.slice(0, 6) === 'https:' ? https : http;
    const request = lib.get(url, { headers: { 'User-Agent': 'modelwrite-mcp-installer' } }, (response) => {
      const status = response.statusCode || 0;

      if (status >= 300 && status < 400 && response.headers.location) {
        response.resume();
        if (redirects > 5) {
          reject(new Error('too many redirects'));
          return;
        }
        const next = new URL(response.headers.location, url).toString();
        download(next, dest, redirects + 1).then(resolve, reject);
        return;
      }

      if (status !== 200) {
        response.resume();
        const hint = status === 404
          ? ' (release v' + VERSION + ' or asset ' + asset + ' not found)'
          : '';
        reject(new Error('HTTP ' + status + hint));
        return;
      }

      // Download to a temp name and rename into place only once complete, so a
      // truncated download can never masquerade as a working binary.
      const tmp = dest + '.download-' + process.pid;
      const out = fs.createWriteStream(tmp, { mode: 0o755 });
      response.pipe(out);
      out.on('finish', () => {
        out.close(() => {
          fs.renameSync(tmp, dest);
          resolve();
        });
      });
      out.on('error', (err) => {
        try { fs.unlinkSync(tmp); } catch (_) {}
        reject(err);
      });
      response.on('error', (err) => {
        out.destroy();
        try { fs.unlinkSync(tmp); } catch (_) {}
        reject(err);
      });
    });

    request.on('error', reject);
    request.setTimeout(120000, () => {
      request.destroy(new Error('download timed out after 120s'));
    });
  });
}
