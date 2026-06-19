import { test, expect } from './fixtures';

const mockAccount = '0xdeC0ded0000000000000000000000000000001193';

test('extension service worker loads', async ({ context, extensionId }) => {
  const worker = context
    .serviceWorkers()
    .find((serviceWorker) => serviceWorker.url().startsWith(`chrome-extension://${extensionId}/`));

  expect(extensionId).toMatch(/^[a-p]{32}$/);
  expect(worker?.url()).toBe(`chrome-extension://${extensionId}/background.js`);
});

test('local dapp can connect through the injected provider', async ({ page }, testInfo) => {
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
    requestAccounts: [mockAccount],
    accountsAfter: [mockAccount],
    chainId: '0xaa36a7',
    connectedAfter: true,
    events: [{ name: 'accountsChanged', payload: [mockAccount] }],
    removedEvents: [],
    isDeckard: true,
    selectedAddress: mockAccount,
  });
  expect(pageErrors).toEqual([]);

  await page.evaluate((state) => {
    const output = document.querySelector('#output');
    if (output) {
      output.textContent = JSON.stringify(state, null, 2);
    }
  }, providerState);

  await page.screenshot({
    path: testInfo.outputPath('connected-dapp.png'),
    fullPage: true,
  });
});

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
