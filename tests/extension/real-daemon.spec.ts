import { expect, extensionTest as test } from './extension-fixtures';
import {
  realDaemonAccount,
  realDaemonChainId,
  requestBridge,
  startRealDaemonBridge,
} from './real-daemon-harness';

test.describe.configure({ mode: 'serial' });

test('local dapp can connect through the extension to a real unlocked daemon', async ({
  page,
}, testInfo) => {
  const runtime = await startRealDaemonBridge('unlocked', testInfo.title);
  try {
    const pageErrors: string[] = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') {
        pageErrors.push(message.text());
      }
    });

    await page.goto('/');
    await expect(page.locator('#output')).toContainText('window.ethereum detected');

    const providerState = await page.evaluate(async () => {
      const provider = window.ethereum;
      if (!provider) {
        throw new Error('window.ethereum missing');
      }
      const accountsBefore = await provider.request({ method: 'eth_accounts' });
      const requestAccounts = await provider.request({ method: 'eth_requestAccounts' });
      const accountsAfter = await provider.request({ method: 'eth_accounts' });
      const chainId = await provider.request({ method: 'eth_chainId' });
      return {
        accountsBefore,
        requestAccounts,
        accountsAfter,
        chainId,
        isDeckard: Boolean(provider.isDeckard),
        selectedAddress: provider.selectedAddress,
      };
    });

    expect(providerState).toEqual({
      accountsBefore: [],
      requestAccounts: [realDaemonAccount],
      accountsAfter: [realDaemonAccount],
      chainId: realDaemonChainId,
      isDeckard: true,
      selectedAddress: realDaemonAccount,
    });
    expect(pageErrors).toEqual([]);

    await page.evaluate((state) => {
      const output = document.querySelector('#output');
      if (output) {
        output.textContent = JSON.stringify(state, null, 2);
      }
    }, providerState);

    await page.screenshot({
      path: testInfo.outputPath('real-daemon-connected-dapp.png'),
      fullPage: true,
    });
  } finally {
    await runtime.stop();
  }
});

for (const failure of [
  {
    scenario: 'locked' as const,
    name: 'locked wallet',
    message: 'the wallet is locked',
  },
  {
    scenario: 'missing-socket' as const,
    name: 'missing daemon socket',
    message: 'could not connect to the Deckard signer daemon',
  },
  {
    scenario: 'wrong-chain' as const,
    name: 'wrong chain configuration',
    message: 'different chain',
  },
]) {
  test(`real daemon bridge reports ${failure.name}`, async ({}, testInfo) => {
    const runtime = await startRealDaemonBridge(failure.scenario, testInfo.title);
    try {
      const chainId = await requestBridge('eth_chainId', 1);
      expect(chainId.result).toBe(realDaemonChainId);

      const accountsBefore = await requestBridge('eth_accounts', 2);
      expect(accountsBefore.result).toEqual([]);

      const requestAccounts = await requestBridge('eth_requestAccounts', 3);
      expect(requestAccounts.result).toBeUndefined();
      expect(requestAccounts.error).toMatchObject({
        code: 4900,
      });
      expect(requestAccounts.error.message).toContain(failure.message);
    } finally {
      await runtime.stop();
    }
  });
}

declare global {
  interface Window {
    ethereum?: {
      isDeckard?: boolean;
      selectedAddress?: string | null;
      request(args: { method: string; params?: unknown[] }): Promise<unknown>;
    };
  }
}
