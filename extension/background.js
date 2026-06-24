const SUPPORTED_METHODS = new Set([
  'eth_chainId',
  'eth_accounts',
  'eth_requestAccounts',
  'personal_sign',
  'eth_signTypedData_v4',
  'eth_sign',
  'eth_sendTransaction',
]);

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || message.type !== 'deckard:eip1193-request') {
    return false;
  }

  if (!SUPPORTED_METHODS.has(message.method)) {
    sendResponse({
      id: message.id,
      error: {
        code: 4200,
        message: `Deckard extension does not support ${message.method}`,
      },
    });
    return false;
  }

  const origin = sender.origin || new URL(sender.url || 'http://unknown.invalid').origin;
  fetch('http://127.0.0.1:8765/rpc', {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-deckard-origin': origin,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: message.id,
      method: message.method,
      params: message.params ?? [],
    }),
  })
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(`Deckard bridge HTTP ${response.status}`);
      }
      return response.json();
    })
    .then((payload) => {
      if (payload.error) {
        sendResponse({ id: message.id, error: payload.error });
      } else {
        sendResponse({ id: message.id, result: payload.result });
      }
    })
    .catch((error) => {
      sendResponse({
        id: message.id,
        error: {
          code: 4900,
          message: error instanceof Error ? error.message : String(error),
        },
      });
    });

  return true;
});
