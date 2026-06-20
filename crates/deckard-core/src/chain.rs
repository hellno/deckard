//! The chain capability registry — ONE source of truth for per-chain config + trust tier,
//! keyed by EIP-155 `chain_id`. Before this module a chain's identity was smeared across five
//! unrelated `match chain_id` sites (token list, CoW base, default RPC, the native-asset hero
//! row, the fork/network label); each is now a registry read.
//!
//! ## Standards basis
//!
//! The [`ChainSpec`] shape deliberately mirrors **EIP-3085** (`wallet_addEthereumChain`) and the
//! `ethereum-lists/chains` registry, so a future `wallet_addEthereumChain` surface is a thin map:
//! `network_name` ← chainName, `default_rpc` ← rpcUrls[0], `explorer_url` ← blockExplorerUrls[0],
//! [`NativeAsset`] ← nativeCurrency `{name, symbol, decimals}` (optional, so a gas-less chain like
//! Tempo is `None`). On top of that transport descriptor we add two Deckard-specific trust axes
//! EIP-3085 has no field for: the verified-reads tier ([`Verification`]) and the real-value
//! guardrail classification ([`is_real_value_chain`]).
//!
//! ## Two structural invariants (not by-convention)
//!
//! 1. **Only mainnet may be [`Verification::Mainnet`].** DESIGN.md "Per-chain trust tiers": mainnet
//!    is the only Helios-verified tier; every other chain reads NOT VERIFIED. The registry is a
//!    *closed* const table (no public constructor), and [`tests::mainnet_is_the_only_verified_tier`]
//!    pins that no `chain_id != 1` entry can ever carry `Mainnet`. A non-mainnet chain wearing the
//!    verified look is unrepresentable, not merely discouraged.
//! 2. **[`is_real_value_chain`] is fail-safe.** It is the *negation of a fixed exempt allowlist*,
//!    exposed as a free function that returns `true` (real value, guardrail armed) for any chain
//!    id NOT explicitly exempt — including chains with no registry entry at all. It is never a
//!    per-chain bool a caller can read as `false` off an absent spec. This mirrors the daemon's
//!    `is_testnet_or_fork` (negated); a parity test in `deckard-signerd` pins the two equal so the
//!    #76 guardrail can later delegate here without any behavior change.
//!
//! Per EIP-3085's strongest security clause, the RPC's own `eth_chainId` is **only ever
//! compared-and-rejected** ([`classify_chain_id_probe`]) against the *declared* chain id — it is
//! never used to select a [`ChainSpec`]. Trusting an RPC-reported id to choose config would defeat
//! the EIP-155 replay guarantee a forked/lying RPC is built to exploit.

use crate::tokens::{TokenInfo, DEFAULT_TOKENS, SEPOLIA_TOKENS};

/// EIP-155 chain id for Ethereum mainnet — the only Helios-verified tier.
pub const MAINNET_CHAIN_ID: u64 = 1;
/// EIP-155 chain id for the Sepolia testnet (the `just demo` Sepolia fork preserves this id).
pub const SEPOLIA_CHAIN_ID: u64 = 11_155_111;
/// anvil / hardhat default chain id (the `just qa` vault + most local e2e suites run here).
pub const ANVIL_CHAIN_ID: u64 = 31_337;

/// Chains exempt from the real-value guardrail: public testnets + local dev/fork ids where
/// hands-free agent spend is allowed by default. This is the SINGLE source for core's real-value
/// classification; `deckard-signerd` keeps its own `TESTNET_FORK_CHAIN_IDS` and a parity test pins
/// the two equal, so #76 can later delegate to [`is_real_value_chain`] with no behavior change.
///
/// SAFETY: extend ONLY with testnet / local-dev ids. A mainnet or L2-mainnet id here is a
/// fund-loss fail-open bug. It is an allowlist of ids we trust to move no real value, NOT a proof —
/// a fork could reuse a mainnet id, which is exactly why it is a fixed list, not a heuristic.
const EXEMPT_TESTNET_CHAIN_IDS: &[u64] = &[SEPOLIA_CHAIN_ID, ANVIL_CHAIN_ID];

/// A chain's verified-reads trust tier (DESIGN.md "Per-chain trust tiers").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verification {
    /// Tier 1: Ethereum mainnet, Helios consensus-verified. The ONLY verified tier.
    Mainnet,
    /// Tier 3: a trusted RPC; reads are honestly tagged NOT VERIFIED (`ReadStatus::Unsynced`).
    /// (Tier 2 — sequencer-trusted OP-stack via helios-opstack — is reserved for #77 and is
    /// deliberately absent: we never fake a verified look for a chain we cannot prove.)
    None,
}

/// A chain's native gas asset — mirrors EIP-3085 `nativeCurrency` (`{name, symbol, decimals}`),
/// plus an explicit display `mark`. The mark is carried, not derived: deriving it from the
/// symbol's first character (as the token rows do) would render ETH's hero as `E`, not `Ξ`.
#[derive(Clone, Copy, Debug)]
pub struct NativeAsset {
    /// The single-glyph display mark for the holdings hero row (ETH = `Ξ`).
    pub mark: &'static str,
    /// Human name (EIP-3085 nativeCurrency.name), e.g. "Ethereum".
    pub name: &'static str,
    /// Ticker (EIP-3085 nativeCurrency.symbol), e.g. "ETH".
    pub symbol: &'static str,
    /// ERC-20-style decimals (EIP-3085 nativeCurrency.decimals; a non-negative integer → `u8`).
    pub decimals: u8,
}

/// One chain's capabilities + trust tier. Constructed ONLY inside this module's closed [`REGISTRY`]
/// table — there is no public constructor, which is what makes the "only mainnet is Verified"
/// invariant structural rather than a caller-discipline rule.
#[derive(Clone, Copy, Debug)]
pub struct ChainSpec {
    /// EIP-155 chain id this spec describes (the registry key).
    pub chain_id: u64,
    /// Display name (EIP-3085 chainName), e.g. "Ethereum" / "Sepolia".
    pub network_name: &'static str,
    /// The default JSON-RPC endpoint used when neither env nor settings pick one (EIP-3085
    /// rpcUrls[0]). Resolving this per-chain is what closes the silent-read-mainnet footgun.
    pub default_rpc: &'static str,
    /// Block-explorer base (EIP-3085 blockExplorerUrls[0]); URLs follow the EIP-3091 path scheme.
    pub explorer_url: &'static str,
    /// The native gas asset, or `None` for a chain with no native gas token (e.g. Tempo).
    pub native_asset: Option<NativeAsset>,
    /// The verified-reads trust tier. Invariant: `Mainnet` only ever appears on `chain_id == 1`.
    pub verification: Verification,
    /// Whether Multicall3 is available. It is deployed at the canonical CREATE2 address
    /// `0xcA11bde05977b3631167028862bE2a173976CA11` on every chain that has it (see
    /// `balances::MULTICALL3`), so presence is the only per-chain variable — hence a `bool`, not an
    /// address. A future zkSync-class chain that hosts Multicall3 at a different address would need
    /// this widened to `Option<Address>`.
    pub multicall3: bool,
    /// The CoW Protocol orderbook REST base for this chain, or `None` if CoW has no deployment.
    pub cow_orderbook_base: Option<&'static str>,
    /// Whether Railgun (shielded balances / shield) is available on this chain.
    pub railgun: bool,
    /// The curated, bundled token list for on-chain balance reads (see `tokens` module). Empty for
    /// a chain with no curated list.
    pub tokens: &'static [TokenInfo],
    /// Whether this chain moves real value. Read through the fail-safe free function
    /// [`is_real_value_chain`] (never off an absent spec); the field is the per-chain value the
    /// classifier must agree with (pinned by [`tests::field_matches_fail_safe_classifier`]).
    pub is_real_value_chain: bool,
}

/// Ethereum mainnet (chain 1). The default RPC references [`crate::eth::DEFAULT_RPC`] so the
/// mainnet endpoint has a single source and cannot drift between the two consumers.
const MAINNET: ChainSpec = ChainSpec {
    chain_id: MAINNET_CHAIN_ID,
    network_name: "Ethereum",
    default_rpc: crate::eth::DEFAULT_RPC,
    explorer_url: "https://etherscan.io",
    native_asset: Some(NativeAsset {
        mark: "Ξ",
        name: "Ethereum",
        symbol: "ETH",
        decimals: 18,
    }),
    verification: Verification::Mainnet,
    multicall3: true,
    cow_orderbook_base: Some("https://api.cow.fi/mainnet"),
    railgun: true,
    tokens: DEFAULT_TOKENS,
    is_real_value_chain: true,
};

/// Sepolia (chain 11155111) — a testnet, so NOT verified and NOT real-value. The `just demo`
/// Sepolia fork preserves this id, so the same spec serves both.
const SEPOLIA: ChainSpec = ChainSpec {
    chain_id: SEPOLIA_CHAIN_ID,
    network_name: "Sepolia",
    default_rpc: "https://ethereum-sepolia-rpc.publicnode.com",
    explorer_url: "https://sepolia.etherscan.io",
    native_asset: Some(NativeAsset {
        // Sepolia's native asset is also ETH; the hero renders identically to mainnet today.
        mark: "Ξ",
        name: "Ethereum",
        symbol: "ETH",
        decimals: 18,
    }),
    verification: Verification::None,
    multicall3: true,
    cow_orderbook_base: Some("https://api.cow.fi/sepolia"),
    railgun: true,
    tokens: SEPOLIA_TOKENS,
    is_real_value_chain: false,
};

/// anvil / hardhat local dev (chain 31337). A first-class dev id the app already runs on (`just
/// qa`, e2e suites). No curated tokens, no CoW, no Railgun deployment; native ETH so the holdings
/// hero renders as before. Default RPC is the anvil default so an unset `DECKARD_RPC_URL` on this
/// chain resolves to the local node rather than mainnet.
const ANVIL: ChainSpec = ChainSpec {
    chain_id: ANVIL_CHAIN_ID,
    network_name: "Anvil (local)",
    default_rpc: "http://127.0.0.1:8545",
    explorer_url: "",
    native_asset: Some(NativeAsset {
        mark: "Ξ",
        name: "Ethereum",
        symbol: "ETH",
        decimals: 18,
    }),
    verification: Verification::None,
    multicall3: false,
    cow_orderbook_base: None,
    railgun: false,
    tokens: &[],
    is_real_value_chain: false,
};

/// The closed registry. Adding a chain means adding an entry here and nowhere else; there is no
/// runtime/remote registry (which would re-open the fork-reuses-a-mainnet-id fail-open).
const REGISTRY: &[ChainSpec] = &[MAINNET, SEPOLIA, ANVIL];

/// The [`ChainSpec`] for `chain_id`, or `None` if the chain is not in the registry.
pub fn for_chain(chain_id: u64) -> Option<&'static ChainSpec> {
    REGISTRY.iter().find(|c| c.chain_id == chain_id)
}

/// The display name for `chain_id`, or `None` for an unknown chain.
pub fn network_name(chain_id: u64) -> Option<&'static str> {
    for_chain(chain_id).map(|c| c.network_name)
}

/// The verified-reads trust tier for `chain_id`. Defaults to [`Verification::None`] for an unknown
/// chain — we never assume a verification tier we cannot back (DESIGN.md: never fake the verified
/// look). Only mainnet (chain 1) ever returns [`Verification::Mainnet`].
pub fn verification(chain_id: u64) -> Verification {
    for_chain(chain_id)
        .map(|c| c.verification)
        .unwrap_or(Verification::None)
}

/// The default RPC endpoint for `chain_id`: the registry entry's, else the mainnet default for an
/// unknown chain. The mainnet fallback is intentional and *not* a silent footgun: an unknown
/// `chain_id` paired with the mainnet RPC is caught loudly by the launch probe
/// ([`classify_chain_id_probe`] returns `Mismatch`, and the app refuses to start). A registered
/// non-mainnet chain always resolves to its OWN node, so an unset `DECKARD_RPC_URL` no longer
/// silently reads mainnet.
pub fn default_rpc(chain_id: u64) -> &'static str {
    for_chain(chain_id)
        .map(|c| c.default_rpc)
        .unwrap_or(crate::eth::DEFAULT_RPC)
}

/// Whether `chain_id` moves real value. FAIL-SAFE: every chain is real-value UNLESS it is on the
/// fixed exempt testnet/dev allowlist ([`EXEMPT_TESTNET_CHAIN_IDS`]), so an unknown chain id (no
/// registry entry) is treated as real value and the #76 guardrail stays armed. Mirrors (is the
/// negation of) the daemon's `is_testnet_or_fork`; the two are pinned equal by a parity test in
/// `deckard-signerd`.
pub fn is_real_value_chain(chain_id: u64) -> bool {
    !EXEMPT_TESTNET_CHAIN_IDS.contains(&chain_id)
}

/// The outcome of probing an RPC's `eth_chainId` against the *declared* chain id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainIdProbe {
    /// The RPC's reported chain id equals the declared one — safe to proceed.
    Match,
    /// The RPC is reachable but reports a DIFFERENT chain — a HARD refusal. This is the
    /// silent-wrong-chain footgun (e.g. `DECKARD_CHAIN_ID=11155111` but the RPC is mainnet).
    Mismatch {
        /// The chain id Deckard was configured for.
        declared: u64,
        /// The chain id the RPC actually reported.
        reported: u64,
    },
    /// The RPC could not be reached / did not answer in time. A mismatch cannot be CONFIRMED, so
    /// the caller continues (offline tolerance): if no read ever lands there is no wrong-chain risk.
    Unreachable {
        /// A short, URL-free reason (RPC error text may carry an API key, so it is not surfaced).
        error: String,
    },
}

/// Pure classifier (unit-testable without a live RPC): given the *declared* chain id and the
/// result of an `eth_chainId` fetch, decide the outcome. The RPC-reported id is ONLY ever compared
/// here — never used to (re)select a [`ChainSpec`] — per EIP-3085 ("never use a chain id received
/// from an RPC endpoint to sign"), which is what protects the EIP-155 replay guarantee.
pub fn classify_chain_id_probe(declared: u64, fetched: Result<u64, String>) -> ChainIdProbe {
    match fetched {
        Ok(reported) if reported == declared => ChainIdProbe::Match,
        Ok(reported) => ChainIdProbe::Mismatch { declared, reported },
        Err(error) => ChainIdProbe::Unreachable { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_is_the_only_verified_tier() {
        // The structural DESIGN.md invariant: no non-mainnet chain may wear the verified look.
        for spec in REGISTRY {
            if spec.verification == Verification::Mainnet {
                assert_eq!(
                    spec.chain_id, MAINNET_CHAIN_ID,
                    "chain {} must NOT be Verified — mainnet is the only verified tier",
                    spec.chain_id
                );
            }
        }
        // And mainnet itself IS verified.
        assert_eq!(
            for_chain(MAINNET_CHAIN_ID).map(|c| c.verification),
            Some(Verification::Mainnet)
        );
        // Sepolia (the only shipped non-mainnet real chain) is honestly NOT verified.
        assert_eq!(
            for_chain(SEPOLIA_CHAIN_ID).map(|c| c.verification),
            Some(Verification::None)
        );
    }

    #[test]
    fn unknown_and_exempt_chains_classify_fail_safe() {
        // Unknown ids (incl. chain 0 and an arbitrary L2) are real-value → guardrail armed.
        assert!(is_real_value_chain(0));
        assert!(is_real_value_chain(999_999));
        assert!(is_real_value_chain(8453)); // Base mainnet — not exempt, real value.
        assert!(is_real_value_chain(MAINNET_CHAIN_ID));
        // The exempt testnet/dev ids are the only non-real-value chains.
        assert!(!is_real_value_chain(SEPOLIA_CHAIN_ID));
        assert!(!is_real_value_chain(ANVIL_CHAIN_ID));
    }

    #[test]
    fn field_matches_fail_safe_classifier() {
        // Every registry entry's `is_real_value_chain` field agrees with the free function, so a
        // future hand-edit of one without the other fails CI.
        for spec in REGISTRY {
            assert_eq!(
                spec.is_real_value_chain,
                is_real_value_chain(spec.chain_id),
                "is_real_value_chain field/classifier disagree for chain {}",
                spec.chain_id
            );
        }
    }

    #[test]
    fn token_lists_are_byte_identical_to_the_old_match() {
        // tokens_for must still return exactly the old per-chain arrays (and empty elsewhere).
        // Compare by VALUE, not pointer: `DEFAULT_TOKENS`/`SEPOLIA_TOKENS` are `const`s, so each use
        // may promote to a distinct anonymous static — `ptr::eq` is not a valid identity check here.
        assert_eq!(crate::tokens::tokens_for(MAINNET_CHAIN_ID), DEFAULT_TOKENS);
        assert_eq!(crate::tokens::tokens_for(SEPOLIA_CHAIN_ID), SEPOLIA_TOKENS);
        assert!(crate::tokens::tokens_for(424_242).is_empty());
    }

    #[test]
    fn cow_bases_are_byte_identical_to_the_old_match() {
        assert_eq!(
            for_chain(MAINNET_CHAIN_ID).unwrap().cow_orderbook_base,
            Some("https://api.cow.fi/mainnet")
        );
        assert_eq!(
            for_chain(SEPOLIA_CHAIN_ID).unwrap().cow_orderbook_base,
            Some("https://api.cow.fi/sepolia")
        );
        assert_eq!(for_chain(ANVIL_CHAIN_ID).unwrap().cow_orderbook_base, None);
    }

    #[test]
    fn default_rpc_is_per_chain_and_never_silently_mainnet_for_known_chains() {
        // Mainnet shares its single source with eth::DEFAULT_RPC.
        assert_eq!(default_rpc(MAINNET_CHAIN_ID), crate::eth::DEFAULT_RPC);
        // A registered non-mainnet chain resolves to its OWN node, NOT mainnet (the footgun fix).
        assert_ne!(default_rpc(SEPOLIA_CHAIN_ID), crate::eth::DEFAULT_RPC);
        assert!(default_rpc(SEPOLIA_CHAIN_ID).contains("sepolia"));
        assert_eq!(default_rpc(ANVIL_CHAIN_ID), "http://127.0.0.1:8545");
        // An UNKNOWN chain falls back to mainnet — but the launch probe refuses on the mismatch.
        assert_eq!(default_rpc(424_242), crate::eth::DEFAULT_RPC);
    }

    #[test]
    fn native_assets_render_eth_hero_identically() {
        for id in [MAINNET_CHAIN_ID, SEPOLIA_CHAIN_ID, ANVIL_CHAIN_ID] {
            let n = for_chain(id).unwrap().native_asset.unwrap();
            assert_eq!(
                (n.mark, n.name, n.symbol, n.decimals),
                ("Ξ", "Ethereum", "ETH", 18)
            );
        }
    }

    #[test]
    fn probe_classifier_matches_rejects_and_tolerates_offline() {
        assert_eq!(classify_chain_id_probe(1, Ok(1)), ChainIdProbe::Match);
        assert_eq!(
            classify_chain_id_probe(SEPOLIA_CHAIN_ID, Ok(MAINNET_CHAIN_ID)),
            ChainIdProbe::Mismatch {
                declared: SEPOLIA_CHAIN_ID,
                reported: MAINNET_CHAIN_ID,
            }
        );
        assert_eq!(
            classify_chain_id_probe(1, Err("timed out".into())),
            ChainIdProbe::Unreachable {
                error: "timed out".into()
            }
        );
    }
}
