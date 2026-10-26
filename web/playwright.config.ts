import { defineConfig } from '@playwright/test';

// Browser tests against the real server and the built UI (`npm run build`,
// then `cargo build`): see e2e/serve.mjs for the library they start with.
const port = process.env.E2E_PORT ?? '8151';

export default defineConfig({
  testDir: 'e2e',
  // One server, one library: tests run in order so their edits don't race.
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    // Chrome rather than Playwright's Chromium, which cannot decode H.264.
    channel: 'chrome',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'node e2e/serve.mjs',
    url: `http://127.0.0.1:${port}/api/config`,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
