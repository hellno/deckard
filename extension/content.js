const script = document.createElement('script');
script.src = chrome.runtime.getURL('injected.js');
script.onload = () => script.remove();
(document.documentElement || document.head).appendChild(script);

window.addEventListener('message', (event) => {
  if (event.source !== window) return;
  const message = event.data;
  if (!message || message.type !== 'deckard:eip1193-request') return;

  chrome.runtime.sendMessage(message, (response) => {
    window.postMessage(
      {
        type: 'deckard:eip1193-response',
        id: message.id,
        result: response?.result,
        error: response?.error,
      },
      window.location.origin,
    );
  });
});
