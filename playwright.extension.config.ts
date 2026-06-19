import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/extension',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  reporter: [
    ['list'],
    ['html', { outputFolder: 'playwright-report/extension', open: 'never' }],
  ],
  outputDir: 'test-results/extension',
  use: {
    baseURL: 'http://127.0.0.1:8777',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
    video: 'retain-on-failure',
    serviceWorkers: 'allow',
  },
  webServer: {
    command: 'python3 -m http.server 8777 --directory examples/browser-bridge-dapp',
    url: 'http://127.0.0.1:8777',
    reuseExistingServer: !process.env.CI,
    stdout: 'pipe',
    stderr: 'pipe',
    timeout: 10_000,
  },
});
