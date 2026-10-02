// UI tests: Playwright drives Chromium against the real program on a temporary library
// (start-server.mjs). One worker and files in order, since some tests change the library.
// Run with `npm test` in this folder after `cargo build --release`.

import { defineConfig, devices } from "@playwright/test";

const port = process.env.PORT ?? "7979";

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.mjs",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    ...devices["Desktop Chrome"],
    viewport: { width: 1300, height: 850 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node start-server.mjs",
    url: `http://127.0.0.1:${port}/api/status`,
    env: { PORT: port },
    timeout: 120_000,
    reuseExistingServer: false,
    // A signal rather than a kill, so start-server.mjs deletes the temporary library.
    gracefulShutdown: { signal: "SIGTERM", timeout: 5_000 },
  },
});
