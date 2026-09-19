// SPDX-License-Identifier: AGPL-3.0-or-later
'use strict';

const { defineConfig } = require('@playwright/test');
const { baseURL } = require('./env');

module.exports = defineConfig({
  testDir: './tests',
  // The suite shares one seeded server, so a single worker keeps logs
  // deterministic and resource use low.
  workers: 1,
  retries: 0,
  timeout: 30000,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL,
    trace: 'retain-on-failure',
  },
  globalSetup: require.resolve('./global-setup'),
  globalTeardown: require.resolve('./global-teardown'),
});
