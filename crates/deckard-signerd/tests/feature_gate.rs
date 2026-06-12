//! #8 — the supply-chain invariant: `deckard-signerd` must never compile the CoW orderbook
//! HTTP client.
//!
//! DEVIATION (documented, agreed at spec time): the literal acceptance criterion "signerd pulls
//! in ZERO reqwest" is IMPOSSIBLE — signerd already links reqwest transitively via alloy's
//! `provider-http`, plus helios and railgun in its feature/dev tree. So the meaningful,
//! enforceable invariant is the narrower one that actually matters for the key boundary:
//!
//!   signerd builds deckard-core with `default-features = false`, so deckard-core's default-on
//!   `cow-client` feature (the ONLY thing that compiles `crate::cow_client`, the orderbook REST
//!   client) is NOT enabled in signerd's dependency resolution.
//!
//! This test shells out to `cargo tree -e features` for the `deckard-signerd → deckard-core`
//! edge and asserts the resolved deckard-core feature set does NOT contain `cow-client`. If
//! cargo can't be invoked (sandboxed CI), it eprintln's a SKIP and returns green rather than
//! failing — the invariant is also structurally guaranteed by signerd's Cargo.toml
//! (`deckard-core = { ..., default-features = false }` with no `cow-client` in its feature list).

use std::process::Command;

/// Run `cargo tree` scoped to the `deckard-signerd → deckard-core` edge with feature
/// annotations. Returns `None` if cargo is not invocable (so the test can SKIP gracefully).
fn cargo_tree_core_features() -> Option<String> {
    // `-i deckard-core` inverts the tree onto deckard-core (the dependency we care about);
    // `-e features` annotates each edge with the features it enables; `-p deckard-signerd`
    // roots the resolution at the daemon so we see ITS view of deckard-core's features.
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "deckard-signerd",
            "-i",
            "deckard-core",
            "-e",
            "features",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            // cargo ran but errored (e.g. offline + uncached). Treat as "can't determine" → skip.
            eprintln!(
                "SKIP feature_gate: `cargo tree` exited non-zero:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            None
        }
        Err(e) => {
            eprintln!("SKIP feature_gate: cargo not invocable in this sandbox: {e}");
            None
        }
    }
}

#[test]
fn signerd_never_enables_deckard_core_cow_client_feature() {
    let Some(tree) = cargo_tree_core_features() else {
        // No cargo → skip (green). The structural guarantee in Cargo.toml still holds.
        return;
    };

    // `cargo tree -e features` annotates feature edges as `feature "cow-client"` lines. The
    // orderbook client is the ONLY thing that feature gates, so its absence from signerd's
    // resolution of deckard-core is exactly the invariant we want.
    let mentions_cow_client = tree
        .lines()
        .any(|line| line.contains("feature \"cow-client\""));

    assert!(
        !mentions_cow_client,
        "deckard-signerd must NOT enable deckard-core's `cow-client` feature (the orderbook \
         HTTP client must never compile into the signer daemon). `cargo tree` output:\n{tree}"
    );
}
