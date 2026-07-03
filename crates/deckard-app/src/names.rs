//! Identity naming — the human-readable name a wallet or the agent shows before (and unless) the
//! operator renames it (E2, #182). DESIGN.md §"Identity is named": wallets/agents carry a real
//! name/handle; the literal word Wallet is never an entity label, and the breadcrumb names the
//! entity, never a project prefix.
//!
//! A fresh vault reads like `Meridian`, not `Wallet` or a raw `0x…` address. The wallet name is
//! derived deterministically from the account address (stable across launches for the same key, so
//! it never flickers), and the agent's handle is assigned from a rotating city list (retiring the
//! old fixed placeholder). Both are overridable: the override is a persisted `Settings` field read
//! by `Shell::wallet_name` / `Shell::agent_handle`; this module only supplies the default when no
//! override is set.

/// Curated wallet codenames — calm place/landmark words, never crypto jargon (DESIGN.md §Language).
/// The default name is one of these, chosen deterministically from the account address so the same
/// key always reads the same, yet two different wallets read differently. `Meridian` leads so the
/// golden-ref demo account reads like the reference.
const WALLET_NAMES: &[&str] = &[
    "Meridian", "Harbor", "Vantage", "Beacon", "Cascade", "Summit", "Anchor", "Haven", "Compass",
    "Vista", "Keystone", "Bastion",
];

/// Auto-assigned agent handles — a rotating city list (DESIGN.md §request-origin model: "non-human
/// sessions get an auto-assigned handle"). Index 0 is the first agent's default (`Kyoto`), which
/// retires the old fixed placeholder handle.
const AGENT_HANDLES: &[&str] = &[
    "Kyoto", "Osaka", "Lisbon", "Oslo", "Nairobi", "Quito", "Bergen", "Cairo",
];

/// A small, dependency-free FNV-1a hash of `seed` — deterministic (never `rand`), so the same
/// address always maps to the same codename across launches.
fn fnv1a(seed: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in seed.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The deterministic default name for the wallet at `address` (its EIP-55 string in practice). An
/// empty seed falls to the first name, so a not-yet-unlocked shell still reads sensibly.
pub(crate) fn default_wallet_name(address: &str) -> &'static str {
    let idx = (fnv1a(address) as usize) % WALLET_NAMES.len();
    WALLET_NAMES[idx]
}

/// The default handle for the agent at `index` (one agent in v1 scope → index 0 = `Kyoto`); wraps
/// if there are ever more agents than curated handles.
pub(crate) fn default_agent_handle(index: usize) -> &'static str {
    AGENT_HANDLES[index % AGENT_HANDLES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_name_is_deterministic_and_curated() {
        let a = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
        // Same address → same name, every call (no rng, no per-launch drift).
        assert_eq!(default_wallet_name(a), default_wallet_name(a));
        assert!(WALLET_NAMES.contains(&default_wallet_name(a)));
        // Never the forbidden literal, never a raw address.
        assert_ne!(default_wallet_name(a), "Wallet");
        assert!(!default_wallet_name(a).starts_with("0x"));
        // The empty (locked) seed still yields a real codename, not a blank.
        assert!(!default_wallet_name("").is_empty());
    }

    #[test]
    fn agent_handle_rotates_and_retires_placeholder() {
        assert_eq!(default_agent_handle(0), "Kyoto");
        // Distinct handles for distinct indices; wraps past the list end.
        assert_ne!(default_agent_handle(0), default_agent_handle(1));
        assert_eq!(
            default_agent_handle(0),
            default_agent_handle(AGENT_HANDLES.len())
        );
    }

    /// Reflective source-scan (mirrors `tokens::lint::no_raw_text_size_px_in_views`): the retired
    /// *entity* labels — the bare word Wallet standing in for the wallet's name, and the old fixed
    /// agent placeholder — must appear NOWHERE as a rendered string literal (E2 acceptance, #182).
    /// Identity is named: the breadcrumb/masthead/sidebar name the entity (`Meridian` / `Kyoto`),
    /// never a mode word.
    ///
    /// The needles are the EXACT quoted tokens (`"Wallet"` / `"Atlas"`), not a substring/word match,
    /// and that precision is deliberate: the DoD bans the word only as an *entity* label ("the
    /// breadcrumb names the entity ... never 'Wallet'"), while DESIGN + the golden ref use the word
    /// descriptively everywhere — the `Wallets` sidebar group, the `This wallet` rail header, the
    /// Settings `Wallet name` field. A substring match would false-positive on all of those; the
    /// standalone `"Wallet"` literal is unambiguously the retired entity label. Comments/docs are
    /// exempt (they explain the rule); this module is exempt (it names the needles).
    #[test]
    fn retired_identity_labels_are_never_rendered() {
        use std::fs;
        use std::path::Path;

        // Build the forbidden needles from chars so THIS file never contains the sequence it bans
        // (`"{q}Wallet{q}"` is not `"Wallet"`), and pair each with a fix hint.
        let q = '"';
        let forbidden = [
            (
                format!("{q}Wallet{q}"),
                "name the entity (e.g. Meridian) via Shell::wallet_name",
            ),
            (
                format!("{q}Atlas{q}"),
                "use the generated handle via Shell::agent_handle (e.g. Kyoto)",
            ),
        ];

        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        for entry in fs::read_dir(&src).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            // This module names the forbidden needles (in the check + messages); skip it.
            if name == "names.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read source");
            for (i, raw) in text.lines().enumerate() {
                let line = raw.trim_start();
                if line.starts_with("//") {
                    continue;
                }
                for (needle, hint) in &forbidden {
                    if line.contains(needle.as_str()) {
                        offenders.push(format!("{name}:{} — {hint}", i + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "a retired identity label is rendered as a literal (DESIGN.md \"Identity is named\"):\n  {}",
            offenders.join("\n  ")
        );
    }
}
