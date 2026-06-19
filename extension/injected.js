(() => {
  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();
  const deckardIconSvg = [
    '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">',
    '<rect width="96" height="96" rx="18" fill="#090b0b"/>',
    '<path d="M28 22h20c16 0 28 11 28 26S64 74 48 74H28V22Zm14 13v26h6c8 0 14-5 14-13s-6-13-14-13h-6Z" fill="#f4d58d"/>',
    '<circle cx="70" cy="26" r="7" fill="#4dd8d8"/>',
    '</svg>',
  ].join('');
  const deckardProviderInfo = Object.freeze({
    uuid: '3f2e4f7c-5e49-4d7d-8e2c-0d9a7c4f1193',
    name: 'Deckard',
    icon: `data:image/svg+xml,${encodeURIComponent(deckardIconSvg)}`,
    rdns: 'com.deckard.wallet',
  });

  class DeckardProvider {
    constructor() {
      this.isDeckard = true;
      this.selectedAddress = null;
      this.chainId = null;
      this._connected = true;
    }

    request(args) {
      if (!args || typeof args.method !== 'string') {
        return Promise.reject(providerError(4100, 'request({ method }) is required'));
      }

      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject, method: args.method });
        window.postMessage(
          {
            type: 'deckard:eip1193-request',
            id,
            method: args.method,
            params: args.params ?? [],
          },
          window.location.origin,
        );
      });
    }

    isConnected() {
      return this._connected;
    }

    on(eventName, listener) {
      if (typeof listener !== 'function') {
        return this;
      }
      const eventListeners = listeners.get(eventName) ?? new Set();
      eventListeners.add(listener);
      listeners.set(eventName, eventListeners);
      return this;
    }

    removeListener(eventName, listener) {
      const eventListeners = listeners.get(eventName);
      if (!eventListeners) {
        return this;
      }
      eventListeners.delete(listener);
      if (eventListeners.size === 0) {
        listeners.delete(eventName);
      }
      return this;
    }
  }

  function providerError(code, message) {
    const error = new Error(message);
    error.code = code;
    return error;
  }

  function emit(eventName, payload) {
    const eventListeners = listeners.get(eventName);
    if (!eventListeners) {
      return;
    }
    for (const listener of [...eventListeners]) {
      try {
        listener(payload);
      } catch (error) {
        setTimeout(() => {
          throw error;
        });
      }
    }
  }

  const provider = new DeckardProvider();
  const providerDetail = Object.freeze({
    info: deckardProviderInfo,
    provider,
  });

  function announceProvider() {
    window.dispatchEvent(
      new CustomEvent('eip6963:announceProvider', {
        detail: providerDetail,
      }),
    );
  }

  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    const message = event.data;
    if (!message || message.type !== 'deckard:eip1193-response') return;

    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);

    if (message.error) {
      if (message.error.code === 4900 && provider._connected) {
        provider._connected = false;
        emit('disconnect', providerError(4900, message.error.message));
      }
      entry.reject(providerError(message.error.code, message.error.message));
      return;
    }

    if (!provider._connected) {
      provider._connected = true;
      emit('connect', { chainId: provider.chainId });
    }

    if (entry.method === 'eth_chainId') {
      const previousChainId = provider.chainId;
      provider.chainId = message.result;
      if (previousChainId && previousChainId !== message.result) {
        emit('chainChanged', message.result);
      }
    }
    if (entry.method === 'eth_requestAccounts' && Array.isArray(message.result)) {
      const previousAddress = provider.selectedAddress;
      const nextAddress = message.result[0] || null;
      provider.selectedAddress = nextAddress;
      if (previousAddress !== nextAddress) {
        emit('accountsChanged', message.result);
      }
    }
    if (entry.method === 'eth_accounts' && Array.isArray(message.result)) {
      const previousAddress = provider.selectedAddress;
      const nextAddress = message.result[0] || null;
      provider.selectedAddress = nextAddress;
      if (previousAddress !== nextAddress) {
        emit('accountsChanged', message.result);
      }
    }
    entry.resolve(message.result);
  });

  Object.defineProperty(window, 'ethereum', {
    value: provider,
    configurable: true,
  });

  window.addEventListener('eip6963:requestProvider', announceProvider);
  announceProvider();
  window.dispatchEvent(new Event('ethereum#initialized'));
})();
