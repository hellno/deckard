import { test as base, chromium, expect, type BrowserContext, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const repoRoot = path.resolve(__dirname, '../..');
export const extensionDir = path.join(repoRoot, 'extension');
export const bridgeUrl = 'http://127.0.0.1:8765/rpc';
export const dappOrigin = 'http://127.0.0.1:8777';

export type ExtensionManifest = {
  action?: { default_popup?: string };
  browser_action?: { default_popup?: string };
};

type ExtensionFixtures = {
  extensionId: string;
  extensionManifest: ExtensionManifest;
  openExtensionPage: (extensionPath: string) => Promise<Page>;
  openPopupPage: () => Promise<Page>;
};

export const extensionTest = base.extend<ExtensionFixtures>({
  context: async ({ headless }, use, testInfo) => {
    validateExtension();
    const userDataDir = path.join(
      repoRoot,
      '.playwright',
      'profiles',
      `worker-${testInfo.workerIndex}-${Date.now()}`,
    );
    fs.mkdirSync(userDataDir, { recursive: true });

    const context = await chromium.launchPersistentContext(userDataDir, {
      channel: 'chromium',
      headless,
      serviceWorkers: 'allow',
      args: [
        `--disable-extensions-except=${extensionDir}`,
        `--load-extension=${extensionDir}`,
      ],
    });

    await use(context);
    await context.close();
  },

  extensionManifest: async ({}, use) => {
    const manifest = JSON.parse(
      fs.readFileSync(path.join(extensionDir, 'manifest.json'), 'utf8'),
    ) as ExtensionManifest;
    await use(manifest);
  },

  extensionId: async ({ context }, use) => {
    const serviceWorker = await extensionServiceWorker(context);
    const extensionId = serviceWorker.url().split('/')[2];
    expect(extensionId).toBeTruthy();
    await use(extensionId);
  },

  openExtensionPage: async ({ context, extensionId }, use) => {
    await use(async (extensionPath: string) => {
      const normalizedPath = extensionPath.replace(/^\/+/, '');
      const page = await context.newPage();
      await page.goto(`chrome-extension://${extensionId}/${normalizedPath}`);
      return page;
    });
  },

  openPopupPage: async ({ extensionManifest, openExtensionPage }, use) => {
    await use(async () => {
      const popupPath =
        extensionManifest.action?.default_popup ??
        extensionManifest.browser_action?.default_popup;
      if (!popupPath) {
        throw new Error('Deckard extension manifest does not define a popup page');
      }
      return openExtensionPage(popupPath);
    });
  },
});

export { expect };

async function extensionServiceWorker(context: BrowserContext) {
  const existing = context
    .serviceWorkers()
    .find((worker) => worker.url().startsWith('chrome-extension://'));
  if (existing) {
    return existing;
  }
  return context.waitForEvent('serviceworker', {
    predicate: (worker) => worker.url().startsWith('chrome-extension://'),
    timeout: 10_000,
  });
}

function validateExtension() {
  const manifestPath = path.join(extensionDir, 'manifest.json');
  for (const required of [manifestPath, path.join(extensionDir, 'background.js')]) {
    if (!fs.existsSync(required)) {
      throw new Error(`missing extension file: ${required}`);
    }
  }
}
