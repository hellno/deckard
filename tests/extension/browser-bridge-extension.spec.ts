import { test, expect } from './fixtures';

const mockAccount = '0xdec0ded000000000000000000000000000001193';

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
    const connectEvents: unknown[] = [];
    const removedEvents: unknown[] = [];
    const removedListener = (payload: unknown) => removedEvents.push(payload);
    provider.once('connect', (payload) => connectEvents.push(payload));
    provider.on('accountsChanged', (payload) => events.push({ name: 'accountsChanged', payload }));
    provider.on('accountsChanged', removedListener);
    provider.removeListener('accountsChanged', removedListener);
    provider.on('disconnect', removedListener);
    provider.removeAllListeners('disconnect');
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
      connectEvents,
      removedEvents,
      isDeckard: Boolean(provider.isDeckard),
      selectedAddress: provider.selectedAddress,
      hasOnce: typeof provider.once === 'function',
      hasRemoveAllListeners: typeof provider.removeAllListeners === 'function',
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
    connectEvents: [{ chainId: '0xaa36a7' }],
    removedEvents: [],
    isDeckard: true,
    selectedAddress: mockAccount,
    hasOnce: true,
    hasRemoveAllListeners: true,
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

test('local dapp can discover Deckard through EIP-6963', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#output')).toContainText('window.ethereum detected');

  const discoveryState = await page.evaluate(async () => {
    type Eip6963ProviderDetail = {
      info: {
        uuid: string;
        name: string;
        icon: string;
        rdns: string;
      };
      provider: NonNullable<Window['ethereum']>;
    };

    const announcedProviders: Eip6963ProviderDetail[] = [];
    window.addEventListener('eip6963:announceProvider', (event) => {
      announcedProviders.push((event as CustomEvent<Eip6963ProviderDetail>).detail);
    });
    window.dispatchEvent(new Event('eip6963:requestProvider'));
    await new Promise((resolve) => setTimeout(resolve, 0));

    const detail = announcedProviders.find((providerDetail) => (
      providerDetail.info.rdns === 'com.deckard.wallet'
    ));
    if (!detail) {
      throw new Error('Deckard EIP-6963 provider announcement missing');
    }

    const chainId = await detail.provider.request({ method: 'eth_chainId' });
    return {
      announcementCount: announcedProviders.length,
      detailFrozen: Object.isFrozen(detail),
      infoFrozen: Object.isFrozen(detail.info),
      sameProvider: detail.provider === window.ethereum,
      chainId,
      info: detail.info,
    };
  });

  expect(discoveryState).toEqual({
    announcementCount: 1,
    detailFrozen: true,
    infoFrozen: true,
    sameProvider: true,
    chainId: '0xaa36a7',
    info: {
      uuid: '3f2e4f7c-5e49-4d7d-8e2c-0d9a7c4f1193',
      name: 'Deckard',
      icon: expect.stringMatching(/^data:image\/svg\+xml,/),
      rdns: 'com.deckard.wallet',
    },
  });
});

test('local dapp can request reviewed message signatures in dev mode', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#output')).toContainText('window.ethereum detected');

  const state = await page.evaluate(async (account) => {
    const provider = window.ethereum;
    if (!provider) {
      throw new Error('window.ethereum missing');
    }
    await provider.request({ method: 'eth_requestAccounts' });
    const simple = await provider.request({
      method: 'personal_sign',
      params: ['0x68656c6c6f', account],
    });
    const legacy = await provider.request({
      method: 'personal_sign',
      params: [account, 'hello'],
    });
    const typed = await provider.request({
      method: 'eth_signTypedData_v4',
      params: [
        account,
        {
          domain: {
            name: 'Test Signature App',
            version: '1',
            chainId: 11155111,
            verifyingContract: '0x0000000000000000000000000000000000000000',
          },
          types: {
            EIP712Domain: [
              { name: 'name', type: 'string' },
              { name: 'version', type: 'string' },
              { name: 'chainId', type: 'uint256' },
              { name: 'verifyingContract', type: 'address' },
            ],
            TestMessage: [
              { name: 'purpose', type: 'string' },
              { name: 'message', type: 'string' },
            ],
          },
          primaryType: 'TestMessage',
          message: {
            purpose: 'Educational Testing Only',
            message: 'This signature is for testing purposes only.',
          },
        },
      ],
    });
    let ethSignError: { code?: number; message?: string } | null = null;
    try {
      await provider.request({ method: 'eth_sign', params: [account, '0x1234'] });
    } catch (error) {
      ethSignError = {
        code: typeof error === 'object' && error ? (error as { code?: number }).code : undefined,
        message: error instanceof Error ? error.message : String(error),
      };
    }
    return { simple, legacy, typed, ethSignError };
  }, mockAccount);

  expect(state.simple).toMatch(/^0x[0-9a-f]{130}$/);
  expect(state.legacy).toMatch(/^0x[0-9a-f]{130}$/);
  expect(state.typed).toMatch(/^0x[0-9a-f]{130}$/);
  expect(state.ethSignError).toEqual({
    code: 4200,
    message: expect.stringContaining('raw eth_sign'),
  });
});

test('local dapp can request reviewed send transactions in dev mode', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#output')).toContainText('window.ethereum detected');

  const state = await page.evaluate(async (account) => {
    const provider = window.ethereum;
    if (!provider) {
      throw new Error('window.ethereum missing');
    }
    await provider.request({ method: 'eth_requestAccounts' });
    const nativeHash = await provider.request({
      method: 'eth_sendTransaction',
      params: [{ from: account, to: '0x0000000000000000000000000000000000000001', value: '0x1' }],
    });
    const transferHash = await provider.request({
      method: 'eth_sendTransaction',
      params: [{
        from: account,
        to: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
        data: '0xa9059cbb00000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240',
      }],
    });
    const approveHash = await provider.request({
      method: 'eth_sendTransaction',
      params: [{
        from: account,
        to: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
        data: '0x095ea7b300000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240',
      }],
    });

    let contractCallError: { code?: number; message?: string } | null = null;
    try {
      await provider.request({
        method: 'eth_sendTransaction',
        params: [{
          from: account,
          to: '0x0000000000000000000000000000000000000001',
          data: '0xdeadbeef00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001',
        }],
      });
    } catch (error) {
      contractCallError = {
        code: typeof error === 'object' && error ? (error as { code?: number }).code : undefined,
        message: error instanceof Error ? error.message : String(error),
      };
    }

    return { nativeHash, transferHash, approveHash, contractCallError };
  }, mockAccount);

  expect(state.nativeHash).toMatch(/^0x[0-9a-f]{64}$/);
  expect(state.transferHash).toMatch(/^0x[0-9a-f]{64}$/);
  expect(state.approveHash).toMatch(/^0x[0-9a-f]{64}$/);
  expect(state.contractCallError).toEqual({
    code: 4200,
    message: expect.stringContaining('unsupported transaction calldata'),
  });
});

declare global {
  interface Window {
    ethereum?: {
      isDeckard?: boolean;
      selectedAddress?: string | null;
      isConnected(): boolean;
      on(eventName: string, listener: (payload: unknown) => void): Window['ethereum'];
      once(eventName: string, listener: (payload: unknown) => void): Window['ethereum'];
      removeListener(eventName: string, listener: (payload: unknown) => void): Window['ethereum'];
      removeAllListeners(eventName?: string): Window['ethereum'];
      request(args: { method: string; params?: unknown[] }): Promise<unknown>;
    };
  }
}
