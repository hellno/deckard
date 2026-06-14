# Dapp connectivity — research synthesis (2026-06-14)

> How should Deckard — a self-custodial desktop wallet (macOS + Linux now, mobile possibly
> later) whose key lives in a process-isolated signer daemon behind a policy gate and a native
> clear-signing card — let third-party dapps interact with it? This is the cited evidence base
> behind [`docs/adr/0001-dapp-connectivity-architecture.md`](../adr/0001-dapp-connectivity-architecture.md)
> and the PRD work items tracked in GitHub issues (epic #44). Five parallel research angles, claims ranked by confidence,
> primary sources linked. Where a claim is our own reasoning over the sources it is marked
> **(inference)**.

## The question, reframed onto Deckard's boundary

Deckard already chose its trust model (`THREAT-MODEL.md`): **the key lives in `deckard-signerd`; every
other process is an untrusted *proposer* that can only submit an `Intent`; the daemon's policy gate
decides; the human approves via a native clear-signing card.** `deckard-mcp` is the reference proposer
— key-less, no `propose`-arbitrary, no `resolve`. Every dapp-connectivity option below is therefore
judged on one axis: **does it keep third-party code on the untrusted-proposer side of that boundary,
and what new attack surface does it add to reach the daemon?**

The user's clarified options were:
- **A — embedded in-app browser**: render dapps *inside* Deckard via a bundled webview.
- **B — separate browser extension**: dapps run in the user's own browser; an extension forwards
  requests to Deckard.

Research adds a third transport the question implied but didn't name — **WalletConnect v2** (relay
pairing, no extension, desktop + mobile) — and a crux that sits *under* all of them: **how an
untrusted local proposer reaches the daemon socket without being able to self-approve.**

---

## Angle 1 — Browser extension + bridge transport (the Frame.sh model)

1. **Frame.sh does *not* use Chrome native messaging — it exposes a localhost RPC server
   (`ws://127.0.0.1:1248` / `http://127.0.0.1:1248`) that any browser, CLI, or app connects to; the
   extension is a thin relay tagging requests with origin metadata.** (high) —
   github.com/floating/frame, github.com/floating/frame-extension (read from source).
2. **A localhost RPC port is reachable by *any* local process and by web pages: same-origin policy
   does not stop a page from calling localhost, WebSockets aren't CORS-gated at handshake, and
   DNS-rebinding defeats naive Host checks — so the transport must rely on per-request UI confirmation
   + origin tagging, not transport-level isolation.** (high) —
   github.blog/security/.../localhost-dangers-cors-and-dns-rebinding, portswigger.net/web-security/websockets/cross-site-websocket-hijacking.
3. **Chrome native messaging (the safer bridge) runs an OS-installed stdio host gated by a manifest
   `allowed_origins` (exact `chrome-extension://<id>`, no wildcards); the host runs as the invoking
   user with no browser sandbox.** (high) — developer.chrome.com/docs/extensions/develop/concepts/native-messaging.
4. **Native-host registration is itself an attack surface: manifests live in user-writable locations
   (HKCU / per-user dirs), so any local code running as the user can register or overwrite a host.**
   (high) — developer.chrome.com native-messaging docs; textslashplain.com native-messaging writeup.
5. **MV3 background service workers are ephemeral; a `connectNative()` port keeps the worker alive,
   but this has had cross-version edge-case bugs — long-lived bridges need reconnection logic.**
   (high / medium) — developer.chrome.com service-worker lifecycle; chromium issue 40661802.
6. **The extension is a standing supply-chain target. Documented precedents: "The Great Suspender"
   (>2M users) sold then weaponized via auto-update (2020–21); the Cyberhaven OAuth-publish hijack
   (Dec 2024, ≥35 extensions, ~2.6M users); a fake "Safery: Ethereum Wallet" exfiltrating seed
   phrases; a Trust Wallet extension supply-chain bug (~$7M reported).** (high for the first two;
   medium for the crypto cases) — bleepingcomputer, thehackernews, cyberhaven engineering blog,
   sekoia.io, cybersecuritynews.
7. **Provider injection should use EIP-6963 (event-based multi-wallet discovery), not fight over
   `window.ethereum` (EIP-1193's single global, "last to load wins").** (high) — eips.ethereum.org/EIPS/eip-6963.

**Takeaway:** an extension is **desktop-only** (it does not transfer to mobile), its bridge is either
the writable-registration native-messaging host or the web-reachable localhost port, and the extension
artifact *itself* is a recurring drainer vector. For a security-first wallet, shipping an extension
means owning a high-value supply-chain target in two hostile stores.

---

## Angle 2 — WalletConnect v2 / Reown (relay transport)

8. **WC v2 separates pairing (a symmetric key shared in the `wc:` URI, EIP-1328) from sessions
   (app↔wallet); the relay is a pub/sub network routing by topic.** (high) — specs.walletconnect.com pairing-uri, relay-network.
9. **Session traffic is end-to-end encrypted (ChaCha20-Poly1305 AEAD, X25519 ECDH + HKDF); by design
   the relay cannot read transaction or signing payloads.** (high) — specs.walletconnect.com crypto-keys; docs.walletconnect.network/network.
10. **But the relay *does* see metadata: IP addresses, timing/session duration, topic IDs, and
    subscription patterns — it must, to route.** (medium, inference corroborated) — relay-network spec; independent analyses.
11. **Production self-hosting of a relay is officially unsupported; the default endpoint is
    `relay.walletconnect.org` and a Reown Project ID (with usage analytics) is required.** (high) —
    walletconnect-docs FAQ, docs.reown.com/cloud/relay. *This is in direct tension with Deckard's
    "offline-first, privacy-focused" identity.*
12. **Scope is negotiated via CAIP-25 namespaces (per scope: chains CAIP-2, methods, events,
    accounts CAIP-10) — the protocol-level lever to restrict what a connected dapp may request.**
    (high) — specs.walletconnect.com/sign/namespaces.
13. **Dapp metadata at pairing (name/url/icons) is self-asserted and spoofable; the Verify API is the
    anti-phishing mitigation (domain attestation: MATCH / UNVERIFIED / MISMATCH / THREAT) but is
    explicitly "not bulletproof" and routes more signal to Reown.** (high / medium) —
    specs.walletconnect.com/core/verify, session-proposal.
14. **There is *no maintained Rust wallet-side SDK*: `WalletConnectRust` is only the low-level relay
    client + RPC types; the Sign protocol, pairing, and namespace negotiation must be built in-house
    for a native Rust wallet.** (high) — github.com/WalletConnect/WalletConnectRust, docs.walletconnect.network.
15. **WC works for a desktop wallet with no phone in the loop (URI via QR or same-device deep link)
    and is the only one of the three transports that *also* works on mobile.** (high) — walletkit mobile-linking docs.

**Takeaway:** WC is the most architecturally aligned generic transport (untrusted proposer, E2E
encrypted, protocol-level scoping, desktop **and** mobile, no extension to get hijacked) — but it
imports a centralized relay + Project-ID dependency that conflicts with Deckard's privacy ethos, and a
**non-trivial build cost** because the wallet-side protocol must be implemented in Rust.

---

## Angle 3 — Embedded in-app browser / webview (Option A)

16. **The Rust embedding path (`wry`, under Tauri) uses three different engines: WebView2 (Chromium)
    on Windows, WKWebView (WebKit) on macOS, WebKitGTK on Linux — divergent JS behavior and divergent
    CVE streams to track per OS.** (high) — docs.rs/wry.
17. **WebKitGTK on Linux is not vendor-auto-updated; it patches via distro packages with historic
    multi-month lag (the canonical case: Debian shipping WebKitGTK with ~184 known vulns), and still
    ships critical ACE-class CVEs in 2025 (Ubuntu USN-7817-1, Oct 2025).** (high) —
    blogs.gnome.org/mcatanzaro update-on-webkit-security-updates; lists.ubuntu.com USN-7817-1.
18. **On Linux, wry/WebKitGTK does *not* enable the renderer sandbox by default — the default embedded
    posture is an un-sandboxed renderer in the wallet's process tree.** (high) — github.com/tauri-apps/wry issue 935.
19. **Renderer RCE from merely rendering web content is a recurring, exploited-in-the-wild class — V8
    zero-days through 2024–25 (CVE-2024-5274, -7971, 2025-10585) and WebKit (BLASTPASS); embedding the
    engine pulls that entire TCB next to the keys.** (high) — qualys/threatprotect, thehackernews, esecurityplanet/Citizen Lab.
20. **Tauri's own security docs say "avoid loading remote content" and treat the webview as untrusted;
    a real IPC-bypass CVE (GHSA-57fm-592m-34r7) let embedded remote content reach host commands. An
    in-app dapp browser is, by definition, the pattern the framework warns against.** (high) —
    v2.tauri.app/security, tauri GHSA-57fm-592m-34r7.
21. **Mobile wallets (MetaMask Mobile, Trust, Rabby, Coinbase Wallet, imToken) embed a dapp browser
    only because mobile OSes have no extension model — that rationale does *not* transfer to desktop,
    where the user already has a hardened, auto-updating browser.** (high, inference on transfer) —
    metamask mobile docs + corroborating writeups.

**Takeaway:** embedding a browser engine imports a large, separately-versioned, RCE-prone TCB (three
engines, default-off Linux sandbox, distro-lagged patches) into a key-holding app, to replicate a
pattern that exists on mobile only because mobile lacks the options desktop already has. The one real
upside (WKWebView's sandboxed WebContent on macOS) doesn't generalize to Linux. **Reject.**

---

## Angle 4 — The crux: untrusted local proposer → daemon socket without self-approval

22. **This is the classic confused-deputy problem (Hardy 1988): a privileged program tricked by a
    less-privileged caller into misusing its own authority. Root cause is *ambient authority* — Unix
    uid-based access is exactly such a model.** (high) — dl.acm.org/10.1145/54289.871709; wikipedia confused-deputy / ambient-authority.
23. **The canonical fix is a *capability*: bundle designation + permission into one unforgeable
    reference, so the daemon can tell *where* an "approve" came from.** (high) — capability-myths-demolished (Miller/Yee/Shapiro).
24. **Peer-credential checks (`SO_PEERCRED` / `LOCAL_PEERCRED`) prove only *which uid/pid* connected —
    they cannot distinguish a trusted UI from a malicious proposer when both share the uid.** (high) —
    man7 unix(7). *This is exactly Deckard's `auth.rs` today: `same_uid()` is the only gate.*
25. **`SCM_RIGHTS` fd-passing yields an unforgeable capability: a received fd "behaves as though
    created with dup(2)"; a process cannot fabricate a reference to another's open file description.**
    (high) — man7 unix(7); blog.cloudflare.com/know-your-scm_rights.
26. **A `socketpair()` whose one end is inherited only by a specifically spawned child is private by
    the kernel security model — the cleanest way to give *only* the trusted UI an approval channel.**
    (high) — kernel mechanics; corroborated by US Patent 10,846,152.
27. **macOS/BSD differ: no `SO_PEERCRED` (use `LOCAL_PEERCRED`/`xucred`, no pid; pid via
    `LOCAL_PEERPID`); even macOS's richer audit-token has been spoofable when misused — so the
    *capability* (private fd) is safer than *identity inference* and more portable.** (high / medium) —
    golang issue 27613; hacktricks macos-xpc audit-token attack.
28. **Every hardware/companion signer enforces the same split — request over a promiscuous endpoint,
    approval on a surface the requester cannot drive: Trezor (`trezord` localhost gated by a *signed
    whitelist*, approval on device), Ledger (Secure Screen + ERC-7730 clear signing), GridPlus
    (separate SCE draws the screen), Frame (Electron UI is the confirmation surface).** (high) —
    trezord README, ledger secure-screen, gridplus docs, docs.frame.sh.
29. **Origin is attacker-controllable when relayed through an untrusted proposer; the daemon can at
    best attest the connecting *process*, not the upstream dapp origin — so the UI must always
    clear-sign the *actual effects* and label origin as unverified.** (high, inference) — walletconnect Verify docs; WC monorepo issue 5400.

**Takeaway:** the fix Deckard's own threat model defers ("authenticate the resolver via a socketpair
or cookie handed only to the supervised app process") is *the* prerequisite for any external proposer.
Approval must move to an unforgeable capability the daemon hands the GPUI app it already spawns
(`supervise.rs`); the public proposer socket must stop accepting `Resolve`. This is established
security engineering, not novel.

---

## Angle 5 — Message-signing danger surface, clear-signing standards, per-origin scoping

30. **The real danger a dapp connection opens is *off-chain signatures*, not transactions: in 2024
    drainer thefts, EIP-2612 `permit` signatures = 56.7% and `setOwner` = 31.9% (~88.6% combined) of
    losses; ~$494M stolen.** (high) — scamsniffer 2024 report; bleepingcomputer.
31. **`eth_sign` signs an arbitrary 32-byte hash (true blind-sign); MetaMask disabled it by default
    and is removing it (MIP-3).** (high) — support.metamask.io eth_sign risk; MIP-3.
32. **Permit2 concentrates risk: one phishing signature to the canonical Permit2 contract can drain
    allowances across every token the user ever approved to it; Seaport/order signatures are abused as
    gasless NFT/token drainers.** (high) — eocene permit2 analysis; zengo offline-signatures.
33. **EIP-712 domain separator (`name, version, chainId, verifyingContract, salt`) lets a wallet
    detect chain/contract mismatch — but the spec only says SHOULD/MAY, and a Coinspect study found
    none of 40+ wallets warned on a chainId mismatch.** (high / medium) — eips.ethereum.org/EIPS/eip-712; coinspect blog.
34. **EIP-7702 (Final, live since Pectra May 2025) lets an EOA delegate its code to a contract; its
    own Security Considerations say wallets MUST NOT offer an interface to sign arbitrary delegations —
    "there is no safe way." >97% of observed delegations went to one sweeper bytecode
    ("CrimeEnjoyor").** (high) — eips.ethereum.org/EIPS/eip-7702; coindesk/Wintermute.
35. **ERC-7730 (Draft, Ledger-led, registry under `ethereum/clear-signing-erc7730-registry`) is a JSON
    metadata standard describing contract calls + EIP-712 messages in human terms; wallets fetch
    descriptors keyed to contract addresses. Coverage is a structural gap — the long tail of contracts
    has no descriptor (no authoritative coverage % found).** (high / medium on gap) —
    eips.ethereum.org/EIPS/eip-7730; github.com/ethereum/clear-signing-erc7730-registry.
36. **`wallet_addEthereumChain` / `wallet_switchEthereumChain` (EIP-3085/3326) let a dapp add a custom
    RPC; the spec says sign only with the *user-submitted* chainId, never one returned by the RPC, and
    confirm requester + target chain. Malicious RPCs fake balances to bait transactions.** (high) —
    eips.ethereum.org/EIPS/eip-3085, eip-3326.
37. **Per-origin scoping foundations: EIP-2255 (`wallet_requestPermissions`/`getPermissions`, the
    `{invoker, parentCapability, caveats}` object) and CAIP-25 `wallet_createSession` (per-scope
    accounts × chains × methods). Anti-phishing blocklists ship too (MetaMask `eth-phishing-detect`,
    ~205k domains).** (high / medium on count) — eips.ethereum.org/EIPS/eip-2255; chainagnostic.org CAIP-25; github.com/MetaMask/eth-phishing-detect.

**Takeaway:** any dapp surface MUST treat off-chain signatures as first-class danger and extend
Deckard's clear-signing engine to decode EIP-712 / permit / Permit2 / Seaport, hard-handle EIP-7702
delegation, consume ERC-7730 where available with a safe blind-sign fallback, guard
`wallet_addEthereumChain`, and scope each origin (accounts × chains × methods) with a shipped
blocklist. Deckard already has the seed of this (`SwapOrder`/`SignOrder` EIP-712 signing,
`PendingPayloadView::Approve`).

---

## Cross-cutting conclusions (feed the ADR)

- **Embedded webview: reject** (Angle 3). Largest TCB, worst patch cadence, contradicts the
  hardened-minimal-core ethos; mobile rationale doesn't transfer to desktop.
- **The boundary fix is mandatory and transport-independent** (Angle 4). Resolver authentication
  (capability over a daemon-handed fd) must land before *any* external proposer — it is also
  `THREAT-MODEL.md` residual-risk #1.
- **Clear-signing must grow to off-chain signatures** (Angle 5) regardless of transport — the curated
  native swap/bridge already needs Permit2/EIP-712.
- **For generic connectivity, WalletConnect dominates the extension** (Angles 1–2) on mobile-reach and
  on not-shipping-a-hijackable-artifact, at the cost of a relay dependency (privacy tension) and an
  in-house Rust Sign-protocol build. The extension, if ever built, is a secondary desktop convenience.
- **"Curated dapps first" has a more secure reading than *connecting* to dapps at all**: integrate the
  curated set *natively* (build Intents in Rust, as Shield/Swap already do), and defer generic
  connectivity until the boundary + clear-signing foundations are in and an audit has happened.

## Source index (primary)

EIPs/ERCs: eip-712, eip-1193, eip-2255, eip-3085, eip-3326, eip-6963, eip-7702, erc-7730, EIP-1328
(eips.ethereum.org). WalletConnect: specs.walletconnect.com (crypto-keys, pairing-uri, relay-network,
sign/namespaces, core/verify), docs.reown.com/cloud/relay, github.com/WalletConnect/WalletConnectRust.
Frame: github.com/floating/frame(-extension), docs.frame.sh. Webview: docs.rs/wry,
v2.tauri.app/security, tauri GHSA-57fm-592m-34r7, blogs.gnome.org/mcatanzaro, lists.ubuntu.com
USN-7817-1, github.com/tauri-apps/wry#935. IPC/capability: man7 unix(7),
blog.cloudflare.com/know-your-scm_rights, dl.acm.org/10.1145/54289.871709,
papers.agoric.com capability-myths-demolished, trezord README, ledger secure-screen, gridplus docs.
Signing danger: scamsniffer 2024 drainer report, support.metamask.io, MetaMask MIP-3,
github.com/ethereum/clear-signing-erc7730-registry, github.com/MetaMask/eth-phishing-detect,
coindesk/Wintermute (EIP-7702). Browser security: github.blog localhost-dangers,
portswigger.net cross-site-websocket-hijacking. Supply chain: bleepingcomputer (Great Suspender,
$494M), cyberhaven engineering blog, sekoia.io, thehackernews (Trust Wallet).

> Confidence + inference flags are per-claim above. Items explicitly unverified by the agents: exact
> native-messaging size ceilings; Frame WebSocket auth specifics; ERC-7730 coverage %; the
> Coinspect "40+ wallets" recency; several single-vendor loss figures. Treat those as directional.
