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
    requestAccounts: [mockAccount],
    accountsAfter: [mockAccount],
    chainId: '0xaa36a7',
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
      request(args: { method: string; params?: unknown[] }): Promise<unknown>;
    };
  }
}
