//! Command-palette registry — pure DATA + a pure fuzzy `rank()`. The static
//! list below is the single source of truth for what ⌘K can do; execution lives
//! in palette.rs (which matches on `id`). No GPUI, no IO here — frecency comes in
//! as a borrowed `PaletteUsage`, the nucleo `Matcher` is reused across calls.

use gpui_component::IconName;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};

use crate::palette_usage::PaletteUsage;

/// A palette command — pure DATA. Execution lives in palette.rs (matches on `id`).
/// Not `Copy`: `IconName` is `Clone`-only, and commands are always used by reference.
#[derive(Clone)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,              // static; the primary ranked text
    pub aliases: &'static [&'static str], // synonyms; carry the alternate sense
    pub shortcut: Option<&'static str>,   // e.g. "⌘⇧M"
    pub icon: Option<IconName>,           // curated few only (real Lucide names)
}

/// One ranked, displayable result.
pub struct Ranked {
    pub cmd_index: usize, // index into the `commands` slice passed to rank()
    pub matched_alias: Option<&'static str>, // Some(alias) when an alias outscored the title
    pub positions: Vec<usize>, // char positions to bold in the TITLE (empty if alias matched)
}

/// The static registry. Dynamic display labels for mask/theme/agent are applied
/// by palette.rs at render time; ranking always uses these static titles + aliases,
/// so the alternate sense (e.g. "show" for the masked state) stays reachable via
/// aliases regardless of current state.
pub const COMMANDS: &[Command] = &[
    Command {
        id: "portfolio",
        title: "Go to Portfolio",
        aliases: &["home", "balance", "wallet"],
        shortcut: None,
        icon: None,
    },
    Command {
        id: "receive",
        title: "Receive",
        aliases: &["deposit", "address", "qr"],
        shortcut: None,
        icon: None,
    },
    Command {
        id: "shield",
        title: "Shield to private",
        aliases: &["private", "hide", "railgun", "0zk", "shield"],
        shortcut: None,
        icon: None, // no shield glyph in the bundled subset
    },
    Command {
        id: "settings",
        title: "Settings",
        aliases: &["preferences", "config"],
        shortcut: Some("⌘,"),
        icon: Some(IconName::Settings),
    },
    Command {
        id: "copy",
        title: "Copy address",
        aliases: &["copy", "addr", "clipboard"],
        shortcut: None,
        icon: Some(IconName::Copy),
    },
    Command {
        id: "theme",
        title: "Toggle theme",
        aliases: &["dark", "light", "appearance", "mode"],
        shortcut: Some("⌘⇧D"),
        icon: None, // palette.rs swaps Sun/Moon live
    },
    Command {
        id: "mask",
        title: "Mask balances",
        aliases: &["show", "hide", "privacy", "reveal", "balances"],
        shortcut: Some("⌘⇧M"),
        icon: Some(IconName::EyeOff), // palette.rs swaps Eye/EyeOff + label live
    },
    Command {
        id: "agent",
        title: "Simulate agent activity (demo)",
        aliases: &["demo", "atlas", "pause", "stop", "agent"],
        shortcut: None,
        icon: None, // palette.rs draws the cyan squircle
    },
    Command {
        id: "lock",
        title: "Lock wallet",
        aliases: &["lock", "logout", "sign out"],
        shortcut: None,
        icon: None, // no lock glyph in the bundled subset
    },
];

/// Two fuzzy scores count as "near-equal" within this gap; only then does
/// frecency break the tie. Keeps a clearly-better match from being demoted by a
/// popular-but-worse one (nucleo scores run in the dozens-to-hundreds).
const SCORE_TIE_EPSILON: u32 = 8;

/// Rank for `query`:
///  - empty query → ALL commands, ordered by `usage.frecency(id, now)` desc, then
///    registry order; `positions` empty, `matched_alias` None.
///  - non-empty → fuzzy score = max over (title, *aliases) via nucleo; drop
///    non-matches; order by score desc with frecency as a gentle tiebreak (never
///    overrides a clearly-better score). `positions` come from the TITLE match only.
pub fn rank(
    query: &str,
    commands: &[Command],
    usage: &PaletteUsage,
    now: u64,
    matcher: &mut Matcher,
) -> Vec<Ranked> {
    if query.is_empty() {
        // Empty palette: frecency desc, registry order as the stable tiebreak.
        let mut order: Vec<usize> = (0..commands.len()).collect();
        order.sort_by(|&a, &b| {
            let fa = commands.get(a).map_or(0.0, |c| usage.frecency(c.id, now));
            let fb = commands.get(b).map_or(0.0, |c| usage.frecency(c.id, now));
            // Descending frecency; stable sort keeps registry order on ties.
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        return order
            .into_iter()
            .map(|cmd_index| Ranked {
                cmd_index,
                matched_alias: None,
                positions: Vec::new(),
            })
            .collect();
    }

    let pat = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);

    // Per-command scratch reused across the title + alias probes for that command.
    let mut buf: Vec<char> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();

    struct Scored {
        cmd_index: usize,
        score: u32,
        matched_alias: Option<&'static str>,
        positions: Vec<usize>,
        frecency: f32,
    }

    let mut scored: Vec<Scored> = Vec::new();

    for (cmd_index, cmd) in commands.iter().enumerate() {
        // Title probe — capture its match positions for highlighting.
        idx.clear();
        let title_score = pat.indices(Utf32Str::new(cmd.title, &mut buf), matcher, &mut idx);
        // ASCII titles ⇒ char index == byte index; positions index the title's chars.
        let title_positions: Vec<usize> = idx.iter().map(|&p| p as usize).collect();

        // Best score + the source that produced it; ties keep the title (so we
        // surface the title's bold positions rather than tagging it an alias).
        let mut best_score = title_score;
        let mut best_alias: Option<&'static str> = None;

        for &alias in cmd.aliases {
            idx.clear(); // probe alias; we don't keep its positions (title-only highlight)
            if let Some(s) = pat.indices(Utf32Str::new(alias, &mut buf), matcher, &mut idx) {
                if best_score.is_none_or(|b| s > b) {
                    best_score = Some(s);
                    best_alias = Some(alias);
                }
            }
        }

        if let Some(score) = best_score {
            // When an alias strictly won, the title may not even have matched;
            // report the alias and leave `positions` empty per the contract.
            let (matched_alias, positions) = match best_alias {
                Some(a) => (Some(a), Vec::new()),
                None => (None, title_positions),
            };
            scored.push(Scored {
                cmd_index,
                score,
                matched_alias,
                positions,
                frecency: usage.frecency(cmd.id, now),
            });
        }
    }

    // Score desc; frecency only breaks near-equal scores (gentle tiebreak). A
    // clearly-better fuzzy match (> epsilon) always wins regardless of frecency.
    scored.sort_by(|a, b| {
        let diff = a.score.abs_diff(b.score);
        if diff > SCORE_TIE_EPSILON {
            return b.score.cmp(&a.score);
        }
        b.frecency
            .partial_cmp(&a.frecency)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Final fallback: higher raw score, then stable registry order.
            .then_with(|| b.score.cmp(&a.score))
    });

    scored
        .into_iter()
        .map(|s| Ranked {
            cmd_index: s.cmd_index,
            matched_alias: s.matched_alias,
            positions: s.positions,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleo_matcher::Config;

    // The fuzzy ordering tests run with an empty usage store, so `frecency` is a
    // flat 0.0 across commands — they assert the matcher's ordering, not frecency.
    // If `PaletteUsage` ever lacks a public default ctor this would need a stub;
    // it exposes `load()`, which on a fresh/missing file yields an empty store.
    fn empty_usage() -> PaletteUsage {
        PaletteUsage::load()
    }

    fn matcher() -> Matcher {
        Matcher::new(Config::DEFAULT)
    }

    fn id_at(results: &[Ranked], pos: usize) -> &'static str {
        COMMANDS[results[pos].cmd_index].id
    }

    #[test]
    fn sh_ranks_shield_above_settings() {
        let mut m = matcher();
        let usage = empty_usage();
        let results = rank("sh", COMMANDS, &usage, 0, &mut m);

        let shield = results
            .iter()
            .position(|r| COMMANDS[r.cmd_index].id == "shield");
        let settings = results
            .iter()
            .position(|r| COMMANDS[r.cmd_index].id == "settings");
        assert!(shield.is_some(), "shield must match \"sh\"");
        // "Shield" matches "sh" as a leading-boundary prefix; "Settings" has no
        // 'h', so settings only survives if an alias matches (it does not) ⇒
        // shield always outranks settings (here settings is absent entirely).
        if let (Some(sh), Some(se)) = (shield, settings) {
            assert!(sh < se, "shield must rank above settings for \"sh\"");
        }
        // shield is the top hit. Only assert strict top-1 on a verifiably-empty
        // usage store: with saved frecency, `mask` (alias "show") could tie-break
        // ahead within the score epsilon on a dogfooded machine.
        let all_unseen = COMMANDS.iter().all(|c| usage.frecency(c.id, 0) == 0.0);
        if all_unseen {
            assert_eq!(id_at(&results, 0), "shield");
        }
    }

    #[test]
    fn show_matches_mask_via_alias() {
        let mut m = matcher();
        let usage = empty_usage();
        let results = rank("show", COMMANDS, &usage, 0, &mut m);

        let mask = results
            .iter()
            .find(|r| COMMANDS[r.cmd_index].id == "mask")
            .expect("\"show\" must match the mask command via its \"show\" alias");
        // Alias match ⇒ flagged as such, with no title positions to bold.
        assert_eq!(mask.matched_alias, Some("show"));
        assert!(mask.positions.is_empty());
    }

    #[test]
    fn empty_query_returns_all_nine() {
        let mut m = matcher();
        let usage = empty_usage();
        let results = rank("", COMMANDS, &usage, 0, &mut m);

        assert_eq!(results.len(), COMMANDS.len());
        assert_eq!(COMMANDS.len(), 9);
        for r in &results {
            assert!(r.matched_alias.is_none());
            assert!(r.positions.is_empty());
        }
        // Every command is present exactly once. We assert membership rather than
        // a fixed order: ordering is by frecency, and `load()` reads the real
        // config dir, so a dogfooded machine may carry saved usage. The frecency
        // ordering itself is exercised by `frecency_orders_empty_query` below,
        // which is gated on a verifiably-empty store.
        let mut ids: Vec<&str> = results.iter().map(|r| COMMANDS[r.cmd_index].id).collect();
        ids.sort_unstable();
        let mut expected: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    #[test]
    fn frecency_orders_empty_query() {
        let mut m = matcher();
        let usage = empty_usage();
        // Only meaningful when `load()` actually yielded an empty store; on a
        // machine with saved palette usage we skip rather than assert a false
        // expectation. (No public empty constructor on PaletteUsage to force it.)
        let all_unseen = COMMANDS.iter().all(|c| usage.frecency(c.id, 0) == 0.0);
        if !all_unseen {
            return;
        }
        let results = rank("", COMMANDS, &usage, 0, &mut m);
        // Flat frecency ⇒ stable registry order.
        assert_eq!(id_at(&results, 0), "portfolio");
        assert_eq!(id_at(&results, COMMANDS.len() - 1), "lock");
    }

    #[test]
    fn junk_query_returns_empty() {
        let mut m = matcher();
        let usage = empty_usage();
        let results = rank("zzzz", COMMANDS, &usage, 0, &mut m);
        assert!(results.is_empty());
    }
}
