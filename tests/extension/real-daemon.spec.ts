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
      const events: Array<{ name: string; payload: unknown }> = [];
      const removedEvents: unknown[] = [];
      const removedListener = (payload: unknown) => removedEvents.push(payload);
      provider.on('accountsChanged', (payload) => events.push({ name: 'accountsChanged', payload }));
      provider.on('accountsChanged', removedListener);
      provider.removeListener('accountsChanged', removedListener);
      const accountsBefore = await provider.request({ method: 'eth_accounts' });
      const connectedBefore = provider.isConnected();
      const requestAccounts = await provider.request({ method: 'eth_requestAccounts' });
      const accountsAfter = await provider.request({ method: 'eth_accounts' });
      const chainId = await provider.request({ method: 'eth_chainId' });
      return {
        accountsBefore,
        connectedBefore,
        requestAccounts,
        accountsAfter,
        chainId,
        connectedAfter: provider.isConnected(),
        events,
        removedEvents,
        isDeckard: Boolean(provider.isDeckard),
        selectedAddress: provider.selectedAddress,
      };
    });

    expect(providerState).toEqual({
      accountsBefore: [],
      connectedBefore: true,
      requestAccounts: [realDaemonAccount],
      accountsAfter: [realDaemonAccount],
      chainId: realDaemonChainId,
      connectedAfter: true,
      events: [{ name: 'accountsChanged', payload: [realDaemonAccount] }],
      removedEvents: [],
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
  test(`real daemon bridge reports ${failure.name}`, async ({ page }, testInfo) => {
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

      await page.goto('/');
      await expect(page.locator('#output')).toContainText('window.ethereum detected');

      const providerState = await page.evaluate(async () => {
        const provider = window.ethereum;
        if (!provider) {
          throw new Error('window.ethereum missing');
        }
        const disconnects: Array<{ code?: number; message?: string }> = [];
        provider.on('disconnect', (error) => {
          const providerError = error as Error & { code?: number };
          disconnects.push({
            code: error instanceof Error ? providerError.code : undefined,
            message: error instanceof Error ? providerError.message : undefined,
          });
        });
        let requestError: { code?: number; message?: string } | undefined;
        try {
          await provider.request({ method: 'eth_requestAccounts' });
        } catch (error) {
          const providerError = error as Error & { code?: number };
          requestError = {
            code: error instanceof Error ? providerError.code : undefined,
            message: error instanceof Error ? providerError.message : undefined,
          };
        }
        return {
          connected: provider.isConnected(),
          disconnects,
          requestError,
        };
      });

      expect(providerState.connected).toBe(false);
      expect(providerState.disconnects).toEqual([
        expect.objectContaining({
          code: 4900,
        }),
      ]);
      expect(providerState.disconnects[0]?.message).toContain(failure.message);
      expect(providerState.requestError).toMatchObject({
        code: 4900,
      });
      expect(providerState.requestError?.message).toContain(failure.message);
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
      isConnected(): boolean;
      on(eventName: string, listener: (payload: unknown) => void): Window['ethereum'];
      removeListener(eventName: string, listener: (payload: unknown) => void): Window['ethereum'];
      request(args: { method: string; params?: unknown[] }): Promise<unknown>;
    };
  }
}
