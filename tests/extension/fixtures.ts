import { test as base, chromium, expect, type BrowserContext, type Page } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '../..');
const extensionDir = path.join(repoRoot, 'extension');
const bridgeUrl = 'http://127.0.0.1:8765/rpc';
const dappOrigin = 'http://127.0.0.1:8777';
const mockAccount = '0xdeC0ded0000000000000000000000000000001193';

type ExtensionManifest = {
  action?: { default_popup?: string };
  browser_action?: { default_popup?: string };
};

type ExtensionFixtures = {
  extensionId: string;
  extensionManifest: ExtensionManifest;
  openExtensionPage: (extensionPath: string) => Promise<Page>;
  openPopupPage: () => Promise<Page>;
};

type WorkerFixtures = {
  bridgeEndpoint: string;
};

export const test = base.extend<ExtensionFixtures, WorkerFixtures>({
  bridgeEndpoint: [
    async ({}, use) => {
      const bridge = await ensureBridge();
      await use(bridgeUrl);
      await bridge?.stop();
    },
    { scope: 'worker', auto: true },
  ],

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

type BridgeHandle = {
  stop: () => Promise<void>;
};

async function ensureBridge(): Promise<BridgeHandle | undefined> {
  if (await bridgeReady()) {
    if (process.env.DECKARD_QA_REUSE_BRIDGE === '1') {
      return undefined;
    }
    throw new Error(
      'Deckard browser bridge is already listening on 127.0.0.1:8765; stop it or set DECKARD_QA_REUSE_BRIDGE=1',
    );
  }

  const child = spawn(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '-p',
      'deckard-browser-bridge',
      '--no-default-features',
      '--',
      '--bind',
      '127.0.0.1:8765',
      '--dev-mock-account',
      mockAccount,
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        DECKARD_CHAIN_ID: '11155111',
      },
    },
  );

  const logs: string[] = [];
  collectBridgeLogs(child, logs);
  await waitForBridge(child, logs);

  return {
    stop: () => stopBridge(child),
  };
}

function collectBridgeLogs(child: ChildProcessWithoutNullStreams, logs: string[]) {
  const collect = (chunk: Buffer) => {
    logs.push(chunk.toString());
    if (logs.length > 30) {
      logs.shift();
    }
  };
  child.stdout.on('data', collect);
  child.stderr.on('data', collect);
}

async function waitForBridge(child: ChildProcessWithoutNullStreams, logs: string[]) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Deckard browser bridge exited early:\n${logs.join('')}`);
    }
    if (await bridgeReady()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Deckard browser bridge did not become ready:\n${logs.join('')}`);
}

async function bridgeReady() {
  try {
    const response = await fetch(bridgeUrl, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-deckard-origin': dappOrigin,
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'eth_chainId',
        params: [],
      }),
      signal: AbortSignal.timeout(1_000),
    });
    if (!response.ok) {
      return false;
    }
    const payload = await response.json();
    return payload.result === '0xaa36a7';
  } catch {
    return false;
  }
}

async function stopBridge(child: ChildProcessWithoutNullStreams) {
  if (child.exitCode !== null) {
    return;
  }
  child.kill('SIGTERM');
  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      child.kill('SIGKILL');
      resolve();
    }, 2_000);
    child.once('exit', () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}
