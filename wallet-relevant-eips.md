# Wallet-Relevant EIPs And ERCs

This is Deckard's implementation-oriented shortlist, curated from the canonical
[`ethereum/EIPs`](https://github.com/ethereum/EIPs) and
[`ethereum/ERCs`](https://github.com/ethereum/ERCs) repositories.

Keep rules:

- Keep Final EIPs.
- Keep Final ERCs that appear in `ethereum/EIPs` as moved ERC stubs.
- Keep a small number of Stagnant Interface EIPs only when they are de facto wallet/dapp APIs
  already expected by the ecosystem.
- Drop Draft, Withdrawn, Last Call, purely historical, and not-yet-actionable specs from the
  implementation tables.

Implementation importance is rated for a desktop self-custodial Ethereum wallet whose dapp
communication surface is a browser extension. `Critical` means the extension bridge or
transaction/signing core should support it directly. `High` means expected for a competitive
production wallet. `Medium` means useful or ecosystem-dependent. `Low` means optional.

## Provider, RPC, Permissions, And Dapp Connection

| EIP | Title | Status / Category | Canonical spec | Implementation importance | Wallet relevance |
| ---: | --- | --- | --- | --- | --- |
| 695 | Create `eth_chainId` method for JSON-RPC | Final / Interface | [EIP-695](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-695.md) | Critical | Standard chain identity RPC used for signing, display, and chain switching. |
| 1102 | Opt-in account exposure | De facto / Interface (Stagnant) | [EIP-1102](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1102.md) | Critical | Defines the `eth_requestAccounts` consent-before-account-exposure flow used by browser wallets. |
| 1193 | Ethereum Provider JavaScript API | Final / Interface | [EIP-1193](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1193.md) | Critical | Core injected provider API: `request`, provider errors, account/chain events, and connectivity. |
| 1474 | Remote procedure call specification | De facto / Interface (Stagnant) | [EIP-1474](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1474.md) | Critical | Baseline Ethereum JSON-RPC method, quantity, and error conventions. |
| 2255 | Wallet Permissions System | Final / Interface | [EIP-2255](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2255.md) | High | Defines `wallet_getPermissions` and `wallet_requestPermissions` for restricted wallet methods. |
| 2696 | JavaScript `request` method RPC transport | Final / Interface | [EIP-2696](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2696.md) | Critical | Specifies the JS `request({ method, params })` provider transport shape. |
| 2700 | JavaScript Provider Event Emitter | Final / Interface | [EIP-2700](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2700.md) | Critical | Specifies provider `on` and `removeListener` event behavior. |
| 3085 | `wallet_addEthereumChain` RPC Method | De facto / Interface (Stagnant) | [EIP-3085](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-3085.md) | High | Dapp-requested chain addition with chain metadata and RPC URL validation. |
| 3326 | Wallet Switch Ethereum Chain RPC Method | De facto / Interface (Stagnant) | [EIP-3326](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-3326.md) | High | Defines `wallet_switchEthereumChain` for changing the wallet active chain. |
| 5749 | The `window.evmproviders` object | Final / Interface | [EIP-5749](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-5749.md) | Low | Alternate multi-provider injection model; lower priority than EIP-6963. |
| 5792 | Wallet Call API | Final / Interface | [EIP-5792](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-5792.md) | High | Defines `wallet_sendCalls`, call status, call display, and wallet capabilities. |
| 6963 | Multi Injected Provider Discovery | Final / Interface | [EIP-6963](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-6963.md) | Critical | Current browser discovery mechanism for multiple injected EIP-1193 providers. |

## Signing, Transactions, Fees, And Status

| EIP/ERC | Title | Status / Category | Canonical spec | Implementation importance | Wallet relevance |
| ---: | --- | --- | --- | --- | --- |
| 2 | Homestead Hard-fork Changes | Final / Core | [EIP-2](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2.md) | Critical | Requires low-`s` transaction signatures; signing code must enforce this. |
| 155 | Simple replay attack protection | Final / Core | [EIP-155](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md) | Critical | Defines chain-ID replay protection and legacy transaction `v` rules. |
| 191 | Signed Data Standard | Final / ERC | [ERC-191](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-191.md) | High | Signed-data envelope used by message signing and validator-intended signatures. |
| 658 | Embedding transaction status code in receipts | Final / Core | [EIP-658](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-658.md) | High | Defines transaction receipt success/failure `status` semantics for UI. |
| 712 | Typed structured data hashing and signing | Final / Interface | [EIP-712](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md) | Critical | Defines typed-data hashing, domains, and `eth_signTypedData`. |
| 1271 | Standard Signature Validation Method for Contracts | Final / ERC | [ERC-1271](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-1271.md) | High | Standard signature validation path for smart contract wallets. |
| 1559 | Fee market change for ETH 1.0 chain | Final / Core | [EIP-1559](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1559.md) | Critical | Defines type `0x02` transactions and max fee / priority fee fields. |
| 2098 | Compact Signature Representation | Final / ERC | [ERC-2098](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-2098.md) | Medium | Compact ECDSA signature format wallets may parse or emit. |
| 2681 | Limit account nonce to 2^64-1 | Final / Core | [EIP-2681](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2681.md) | Medium | Bounds valid account nonces for transaction validation. |
| 2718 | Typed Transaction Envelope | Final / Core | [EIP-2718](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2718.md) | Critical | Base envelope for typed transaction formats and receipts. |
| 2930 | Optional access lists | Final / Core | [EIP-2930](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2930.md) | High | Defines type `0x01` access-list transactions and signing payloads. |
| 3607 | Reject transactions from senders with deployed code | Final / Core | [EIP-3607](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-3607.md) | Medium | Wallets should surface or prevent invalid transactions from code-bearing senders. |
| 4844 | Shard Blob Transactions | Final / Core | [EIP-4844](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-4844.md) | Medium | Defines type `0x03` blob transactions, blob fee fields, and signing payloads. |
| 5267 | Retrieval of EIP-712 domain | Final / ERC | [ERC-5267](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-5267.md) | Medium | Domain discovery for safer typed-data verification and display. |
| 6492 | Signature Validation for Predeploy Contracts | Final / ERC | [ERC-6492](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-6492.md) | Medium | Signature validation for counterfactual or not-yet-deployed smart accounts. |
| 7702 | Set Code for EOAs | Final / Core | [EIP-7702](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md) | High | Current EOA code-delegation transaction type for batching, sponsorship, and de-escalation. |
| 7825 | Transaction Gas Limit Cap | Final / Core | [EIP-7825](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7825.md) | Medium | Per-transaction gas cap live with Fusaka; wallets should enforce or surface it. |
| 7951 | Precompile for secp256r1 Curve Support | Final / Core | [EIP-7951](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7951.md) | Medium | Enables device-native P-256/WebAuthn verification patterns for wallets and AA. |

## Account Abstraction, Delegation, Batching, And Sponsorship

| EIP/ERC | Title | Status / Category | Canonical spec | Implementation importance | Wallet relevance |
| ---: | --- | --- | --- | --- | --- |
| 2771 | Secure Protocol for Native Meta Transactions | Final / ERC | [ERC-2771](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-2771.md) | Medium | Trusted-forwarder meta-transaction pattern for relayed wallet flows. |
| 4337 | Account Abstraction Using Alt Mempool | Final / ERC | [ERC-4337](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-4337.md) | High | Core app-layer AA model for UserOperations, EntryPoint, bundlers, and paymasters. |
| 7702 | Set Code for EOAs | Final / Core | [EIP-7702](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-7702.md) | High | EOA code-delegation primitive for smart-wallet features without account migration. |

## Assets, Tokens, NFTs, And Transfer Display

| EIP/ERC | Title | Status / Category | Canonical spec | Implementation importance | Wallet relevance |
| ---: | --- | --- | --- | --- | --- |
| 20 | Token Standard | Final / ERC | [ERC-20](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-20.md) | Critical | Fungible token baseline for balances, transfers, symbols, decimals, and asset lists. |
| 165 | Standard Interface Detection | Final / ERC | [ERC-165](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-165.md) | High | Interface detection for token and NFT capability checks. |
| 721 | Non-Fungible Token Standard | Final / ERC | [ERC-721](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-721.md) | High | NFT ownership, transfers, and metadata display baseline. |
| 747 | `wallet_watchAsset` RPC Method | Final / Interface | [EIP-747](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-747.md) | High | Lets dapps request that a wallet track a token asset. |
| 777 | Token Standard | Final / ERC | [ERC-777](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-777.md) | Low | Alternate fungible token model wallets may need to recognize. |
| 1046 | Token Metadata | Final / ERC | [ERC-1046](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-1046.md) | Medium | Token metadata URI support referenced by wallet asset display flows. |
| 1155 | Multi Token Standard | Final / ERC | [ERC-1155](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-1155.md) | High | Multi-token standard for fungible, semi-fungible, and NFT holdings. |
| 2612 | `permit` Extension for EIP-20 Signed Approvals | Final / ERC | [ERC-2612](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-2612.md) | High | Typed-data token approvals that wallets must display and sign safely. |
| 4906 | EIP-721 Metadata Update Extension | Final / ERC | [ERC-4906](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-4906.md) | Low | NFT metadata update events for refreshing wallet displays. |

## Names, Addresses, URIs, And Identity

| EIP/ERC | Title | Status / Category | Canonical spec | Implementation importance | Wallet relevance |
| ---: | --- | --- | --- | --- | --- |
| 55 | Mixed-case checksum address encoding | Final / ERC | [ERC-55](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-55.md) | Critical | Standard Ethereum address checksum display and validation. |
| 137 | Ethereum Domain Name Service | Final / ERC | [ERC-137](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-137.md) | High | ENS name resolution baseline. |
| 181 | ENS support for reverse resolution | Final / ERC | [ERC-181](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-181.md) | High | Reverse resolution for showing names from addresses. |
| 681 | URL Format for Transaction Requests | Final / ERC | [ERC-681](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-681.md) | Medium | Payment and transaction request URLs for wallet deeplinks. |
| 1328 | WalletConnect Standard URI Format | Final / ERC | [ERC-1328](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-1328.md) | Low | WalletConnect-style URI handling and pairing surface. |
| 3668 | CCIP Read: Secure offchain data retrieval | Final / ERC | [ERC-3668](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-3668.md) | High | Offchain lookup path used by modern ENS and resolver flows. |
| 4361 | Sign-In with Ethereum | Final / ERC | [ERC-4361](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-4361.md) | High | Wallet login and identity message flow. |

## Kohaku Wallet References

These entries are not EIPs/ERCs. Keep them as implementation references for Deckard's privacy and
browser-extension architecture, not as standards targets.

| Category | Title | Source | Wallet relevance | Implementation importance | Additional notes |
| --- | --- | --- | --- | --- | --- |
| Architecture | Privacy-first Ethereum tooling | [kohaku/README.md](https://github.com/ethereum/kohaku/blob/master/README.md) | Positions Kohaku as reusable wallet infrastructure for privacy-preserving Ethereum UX. | High | Production integration still needs security review for each reused package. |
| Dapp bridge | EIP-1193 provider abstraction | [kohaku/crates/eip-1193-provider/README.md](https://github.com/ethereum/kohaku/blob/master/crates/eip-1193-provider/README.md), [kohaku/packages/provider/README.md](https://github.com/ethereum/kohaku/blob/master/packages/provider/README.md) | Useful for keeping the browser extension provider thin while routing requests into native, WASM, Ethers, Viem, Helios, or Colibri backends. | Critical | Fits the desktop-wallet model where the extension is only the dapp transport and the desktop app owns policy, accounts, and signing. |
| Privacy infrastructure | Private RPC and light-client posture | [kohaku/docs/pages/practices.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/practices.mdx), [kohaku/docs/pages/privacy.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/privacy.mdx) | Reduces wallet activity leakage from default RPCs and centralized indexing. | High | Kohaku docs call out user-defined RPCs, Helios-style verification, and optional network privacy routing as wallet best practices. |
| Account UX | Many accounts, many identities | [kohaku/docs/pages/practices.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/practices.mdx), [kohaku/docs/pages/privacy.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/privacy.mdx) | Encourages per-context accounts and easy account creation during dapp connection. | High | This should influence the extension connect flow: choosing or creating an account should be part of the permission grant, not only a global wallet setting. |
| Plugin system | Standardized private-transaction plugin interface | [kohaku/packages/plugins/README.md](https://github.com/ethereum/kohaku/blob/master/packages/plugins/README.md) | Lets the wallet expose shield, transfer, unshield, balance, and broadcast flows across multiple privacy protocols through one host interface. | High | Host responsibilities include storage, network fetch, keystore derivation, and Ethereum provider access; this maps cleanly to a desktop wallet core. |
| Key management | Portable plugin key derivation | [kohaku/packages/plugins/README.md](https://github.com/ethereum/kohaku/blob/master/packages/plugins/README.md) | Allows privacy protocol keys to derive from the wallet mnemonic where supported. | High | Imported plugin key material is not automatically portable, so backup and sync UX must explicitly include plugin state or imported secrets. |
| Shielded assets | Railgun shielded-pool support | [kohaku/docs/pages/railgun/intro.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/railgun/intro.mdx), [kohaku/crates/railgun/README.md](https://github.com/ethereum/kohaku/blob/master/crates/railgun/README.md) | Adds shielding, internal transfer, and unshielding for ERC-20 assets and ETH via WETH-style wrapping. | Medium | Useful as a wallet feature area, but protocol risk, circuit artifacts, indexing, and compliance posture need separate review before default exposure. |
| Account abstraction | ERC-4337 UserOperation kit | [kohaku/crates/userop-kit/README.md](https://github.com/ethereum/kohaku/blob/master/crates/userop-kit/README.md) | Provides UserOperation building and bundler client support for EntryPoint 0.7 and 0.8. | High | Important if the desktop wallet plans smart accounts, paymasters, sponsored transactions, or batched dapp actions. |
| Asset privacy | Local token discovery caution | [kohaku/docs/pages/privacy.mdx](https://github.com/ethereum/kohaku/blob/master/docs/pages/privacy.mdx) | Pushes the wallet away from centralized all-transfer-event indexing that links addresses to lookup behavior. | High | Prefer local indexing, user-configured indexers, light-client verification, or explicit opt-in discovery modes. |

## Excluded Or Deferred

These were present in the source inventory but are not implementation targets for Deckard right now.

| Reason | EIPs/ERCs |
| --- | --- |
| Historical, Withdrawn, or pure reference material | 86, 107, 2711, 2786, 2938, 3074, 5003, 7701, 7980 |
| Stagnant and not a current wallet/dapp API target | 2015, 2256, 2803, 2831, 5345, 5593, 5806, 7039, 7377, 7713, 7867, 7896 |
| Draft or otherwise not yet stable enough for Deckard's implementation shortlist | 3009, 3770, 6900, 7708, 7749, 7851, 7932, 7966, 7997, 8072, 8123, 8130, 8141, 8151, 8164, 8175, 8197, 8202, 8250, 8266, 8272 |
| ERCs not Final in `ethereum/ERCs` | 67, 634, 831, 1191, 1577, 2304, 2544 |
| Non-EIP Kohaku references deferred until audited or production-ready | Privacy Pools, Tornado-style notes, post-quantum account implementation |
