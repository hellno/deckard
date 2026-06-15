(() => {
  let nextId = 1;
  const pending = new Map();

  class DeckardProvider {
    constructor() {
      this.isDeckard = true;
      this.selectedAddress = null;
      this.chainId = null;
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

    on() {
      // Event emitter support is intentionally out of scope for this milestone.
      return this;
    }

    removeListener() {
      return this;
    }
  }

  function providerError(code, message) {
    const error = new Error(message);
    error.code = code;
    return error;
  }

  const provider = new DeckardProvider();

  window.addEventListener('message', (event) => {
    if (event.source !== window) return;
    const message = event.data;
    if (!message || message.type !== 'deckard:eip1193-response') return;

    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);

    if (message.error) {
      entry.reject(providerError(message.error.code, message.error.message));
      return;
    }

    if (entry.method === 'eth_chainId') {
      provider.chainId = message.result;
    }
    if (entry.method === 'eth_requestAccounts' && Array.isArray(message.result)) {
      provider.selectedAddress = message.result[0] || null;
    }
    entry.resolve(message.result);
  });

  Object.defineProperty(window, 'ethereum', {
    value: provider,
    configurable: true,
  });

  window.dispatchEvent(new Event('ethereum#initialized'));
})();
