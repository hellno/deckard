# Spike prompt: the Deckard-native dapp bridge (PRD-04 #1)

> Feed this whole file to a fresh coding agent. It is self-contained. Goal: prove (or disprove) that a
> **first-party, key-less browser connector** can give Deckard universal dapp reach over a wire Deckard
> **owns end-to-end** — no third-party relay, no embedded browser, no store as a trust anchor — and
> settle the two open choices: **(a) the local wire** (native messaging vs hardened localhost) and
> **(b) the self-distribution path** on both Chromium and Firefox. Standalone spike — do **not** touch
> the app crates (`crates/`) or ship anything user-facing.

Executes the first deliverable of [`docs/prd/04-deckard-native-bridge.md`](../docs/prd/04-deckard-native-bridge.md).
Read it, [ADR 0001](../docs/adr/0001-dapp-connectivity-architecture.md), and
[`docs/research/10-dapp-connectivity.md`](../docs/research/10-dapp-connectivity.md) first. The research
section references below (`§N`) point at claims in that file.

## Context you can trust (verified in research — don't re-litigate)

- **Dapps already speak EIP-1193**, so injecting a *standard* provider (announced via **EIP-6963**, not
  fighting over `window.ethereum`) reaches all of them with no per-dapp work (`§7, 21–22`).
- **Native messaging is the leading wire**: an OS-installed stdio host gated by a manifest
  `allowed_origins` (Chrome, exact `chrome-extension://<id>`, no wildcards) / `allowed_extensions`
  (Firefox, the gecko id), running as the user, **not reachable from web pages** (`§3, 12, 15`).
  Manifest locations differ per browser/OS and live in user-writable paths (`§4, 15`).
- **A localhost RPC port (Frame's `ws://127.0.0.1:1248`) is web-reachable** — same-origin policy does
  not stop a page from calling localhost, WebSockets aren't CORS-gated at handshake, and DNS-rebinding
  defeats naive Host checks. If used at all it needs origin-allowlist + token + Host validation, and
  even then it's weaker than native messaging (`§1, 2`).
- **MV3 service workers are ephemeral**; a `connectNative` port keeps the worker alive but has had
  cross-version edge cases — you must reconnect on `onDisconnect` (`§5`).
- **Trust is bounded by design, not by the store.** The connector is key-less: even fully compromised it
  can only *propose*; it can never sign, self-approve, or exfiltrate a key (the daemon owns the key; the
  resolver capability is PRD-01; effects are clear-signed, PRD-02). This is *why* self-signed
  distribution outside a store is acceptable (`§6, 22, 29`).
- The existing **`deckard-mcp`** is the proposer pattern to mirror: a key-less translator speaking the
  `deckard-contract` wire to `deckard-signerd` (`docs/build/30-mcp-shape.md`). The mock daemon in
  `deckard-contract` (`MockSigner`) answers the socket API without a real key.

## The two questions the spike must settle

1. **Wire:** does **native messaging** carry a full EIP-1193 request/response round-trip cleanly on
   **both Chromium and Firefox**, with stable reconnection under MV3 SW eviction? If yes, it wins and
   localhost is not needed. Only if a target browser/setup *blocks* native messaging do we fall back to
   a hardened localhost wire — in which case prove the origin-allowlist + token + Host-check defenses.
2. **Distribution:** can we **self-distribute a Deckard-signed connector** on both browsers without
   relying on a store as the trust anchor? Document the exact path + friction:
   - Firefox: self-hosted signed XPI (AMO-signed but self-distributed) — confirm it installs.
   - Chromium: normal installs lean on the Web Store; document the self-/sideload path (unpacked/dev
     mode, enterprise policy, `.crx` + `update_url`) and its real-world friction for an end user.

## Tasks (do in order)

1. **Stand up a mock host + mock daemon.** A small Rust binary (the spike's `deckard-bridged` stand-in)
   that: (a) speaks the browser **native-messaging stdio framing** (32-bit native-endian length prefix +
   UTF-8 JSON, `§12`) on one side; (b) speaks the `deckard-contract` wire to the **`MockSigner`** (no
   real key) on the other. Map `eth_requestAccounts` → `Address`, `personal_sign` → a (mock) message
   sign, `eth_sendTransaction` → `Propose`/`Status`/`Execute`. Standalone crate under
   `spikes/deckard-bridge/` with its own `[workspace]`; `.gitignore` `target/` + `Cargo.lock`. Depend on
   `deckard-contract` by path for the wire types; **no real key, no real signing, no broadcasting.**
2. **Build a minimal connector** (Chromium MV3 + Firefox MV2/MV3 as needed; separate manifests). It
   injects an EIP-1193 provider, announces it via **EIP-6963** (`rdns` e.g. `sh.deckard`), and relays
   page JSON-RPC → background → native-messaging port → host. Handle MV3 SW lifecycle: reconnect on
   `onDisconnect`, no lost requests.
3. **Drive a real dapp end to end.** Point a simple test page (or a real read-only dapp) at the injected
   provider and complete: `eth_requestAccounts` (returns the mock address) and one `personal_sign`
   (returns a mock signature). Confirm the request reaches the host and the mock daemon, on **Chromium
   and Firefox**.
4. **Stress the MV3 path.** Force service-worker eviction (idle it out / `chrome://serviceworker-internals`)
   mid-session and confirm the next request reconnects and succeeds (`§5`).
5. **Prove the web-unreachability of native messaging.** From an ordinary web page, attempt to reach the
   host directly (there should be no port to reach). Then, for contrast, if you also stand up the
   localhost fallback, show a cross-origin page *can* reach it without the token/Host defenses — and that
   the defenses block it. This is the security evidence for the wire choice.
6. **Self-distribution dry run.** Produce a signed Firefox XPI and install it from a self-hosted URL
   (not AMO listing). Produce the Chromium artifact and document the install path + friction. Record
   exact steps so PRD-04 can reproduce.

## Constraints

- Standalone: `spikes/deckard-bridge/` (Rust host + mock) with its own `[workspace]`; the connector is
  plain JS/TS, no build pipeline beyond what's needed. Do **not** edit `crates/` or the app.
- **Key-less throughout.** The host/connector never hold a key; reads/proposes only against `MockSigner`.
  No real network broadcast.
- Mirror `deckard-mcp`'s no-secret-in-transcript discipline: no key/passphrase/token in any log or
  message (a quick grep assertion is enough for the spike).

## Success criteria (what "done" means)

- A clear **YES/NO** on: *"Does native messaging carry a full EIP-1193 round-trip cleanly on both
  Chromium and Firefox, with reliable reconnection under MV3 eviction?"* — with the evidence.
- A clear **wire recommendation** (native messaging vs localhost) with the security argument (task 5).
- A clear **self-distribution recommendation** with the exact Firefox + Chromium paths and their
  friction (task 6).
- A runnable spike: connector + host + mock daemon completing `eth_requestAccounts` + `personal_sign`.
- A short report feeding PRD-04's "owned wire" + "distribution" sections.

## Report format (return this)

```
<spike_report>
  <wire_verdict>NATIVE MESSAGING / LOCALHOST REQUIRED / BLOCKED</wire_verdict>
  <native_messaging><!-- works on Chromium? Firefox? MV3 reconnection behavior --></native_messaging>
  <web_unreachability><!-- evidence native messaging exposes no web-reachable port; localhost contrast --></web_unreachability>
  <distribution><!-- Firefox self-hosted-signed XPI: works? Chromium self/sideload path + friction --></distribution>
  <eip6963><!-- announced + coexists with another wallet? --></eip6963>
  <what_worked/>
  <what_failed/>
  <recommendation><!-- one paragraph for docs/prd/04-deckard-native-bridge.md --></recommendation>
  <artifacts><!-- spike crate + connector path; how to run on each browser --></artifacts>
</spike_report>
```
