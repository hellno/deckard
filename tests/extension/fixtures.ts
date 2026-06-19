import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';

import { bridgeUrl, dappOrigin, expect, extensionTest, repoRoot } from './extension-fixtures';

type WorkerFixtures = {
  bridgeEndpoint: string;
};

const mockAccount = '0xdeC0ded0000000000000000000000000000001193';

export const test = extensionTest.extend<{}, WorkerFixtures>({
  bridgeEndpoint: [
    async ({}, use) => {
      const bridge = await ensureBridge();
      await use(bridgeUrl);
      await bridge?.stop();
    },
    { scope: 'worker', auto: true },
  ],
});

export { expect };

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
