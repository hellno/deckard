//! Durable daily-spend accounting (issue #108).
//!
//! The signer's daily cap (`Policy::spent_today_wei`) is RAM-only; this module persists it so a
//! restart — crash, OOM, app update, sleep — doesn't silently zero the day's accounting, and so a
//! **reserve-before-sign** survives a crash between signing and the post-broadcast bump.
//!
//! ## Model
//! `effective_spent = committed_wei + reserved_wei`, mirrored into `Policy::spent_today_wei` so the
//! pure cap decision (`deckard_contract::evaluate`) is unchanged. The lifecycle around one
//! `execute`:
//!
//! ```text
//!   reserve(v)   reserved += v ; persist          (BEFORE the signature is released)
//!     broadcast ── Ok      ──> commit(v)   reserved -= v ; committed += v ; persist
//!               ── clean Err ─> release(v)  reserved -= v ; persist   (RPC rejected — nothing moved)
//!               ── timeout  ──> commit(v)   keep it counted           (may have landed — fail safe)
//!     crash (no commit)    ──> reserved stays on disk ──> load() counts it as spent (conservative)
//! ```
//! Exact chain reconciliation (releasing a genuinely-dropped tx's headroom) is deliberately
//! deferred to a post-#72 issue: recovery here only ever makes the cap *tighter*, never looser.
//!
//! ## Window
//! Bound to `(chain_id, account, UTC day)`. The day floor is **forward-only**: a backward
//! wall-clock can't reset the window. A different chain on load is discarded (a different cap); a
//! different account at unlock resets the window (per-account caps). A missing file is a fresh
//! window; a present-but-unparseable file fails closed (fully spent until the next rollover).

use std::path::PathBuf;

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use deckard_core::atomic_write;

/// The on-disk record, JSON-serialized next to `policy.json` as `spend.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SpendRecord {
    chain_id: u64,
    /// The wallet this window accounts for; the zero address until first bound at unlock.
    account: Address,
    /// Forward-only UTC-day floor (days since the Unix epoch).
    day: u64,
    committed_wei: U256,
    reserved_wei: U256,
}

impl SpendRecord {
    fn fresh(chain_id: u64, account: Address, day: u64) -> Self {
        Self {
            chain_id,
            account,
            day,
            committed_wei: U256::ZERO,
            reserved_wei: U256::ZERO,
        }
    }
}

/// How a counter-file load resolved — split out so the fallbacks are unit-testable without a
/// real filesystem.
#[derive(Debug, PartialEq, Eq)]
enum LoadKind {
    /// Parsed cleanly for THIS chain.
    Loaded,
    /// No file — a normal first run; a fresh window applies quietly.
    Missing,
    /// The file is for a different chain id — discarded (a different chain is a different cap).
    WrongChain,
    /// The file EXISTS but did not parse / could not be read (corrupt or hostile write). Fail
    /// closed: fully spent until the next rollover. Loud, like the policy loader's `DefaultInvalid`.
    Invalid(String),
}

/// Pure load resolver: given the raw read result, the daemon's chain, and today's UTC day,
/// produce the in-memory record, a `corrupt` flag, and the classification (for logging/tests).
/// Never panics; an unreadable or unparseable file fails closed (`corrupt = true`).
fn resolve_load(
    read: std::io::Result<Vec<u8>>,
    chain_id: u64,
    today: u64,
) -> (SpendRecord, bool, LoadKind) {
    let fresh = || SpendRecord::fresh(chain_id, Address::ZERO, today);
    match read {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (fresh(), false, LoadKind::Missing),
        Err(e) => (
            fresh(),
            true,
            LoadKind::Invalid(format!("read failed: {e}")),
        ),
        Ok(bytes) => match serde_json::from_slice::<SpendRecord>(&bytes) {
            Ok(rec) if rec.chain_id == chain_id => (rec, false, LoadKind::Loaded),
            Ok(_other_chain) => (fresh(), false, LoadKind::WrongChain),
            Err(e) => (
                fresh(),
                true,
                LoadKind::Invalid(format!("parse failed: {e}")),
            ),
        },
    }
}

/// The durable daily-spend counter. Single-writer (the daemon, under its serializing mutex); every
/// mutation persists atomically (`atomic_write` → fsync + dir-sync).
pub struct SpendStore {
    path: PathBuf,
    chain_id: u64,
    record: SpendRecord,
    /// The file existed but did not load: the cap reads as fully spent (cap exhausted) until the
    /// next rollover or a trusted unlock rebinds a fresh window.
    corrupt: bool,
}

impl SpendStore {
    /// Load at boot. The daemon is `Locked` here, so the account is not yet known — it is bound and
    /// validated later at unlock ([`bind_account`](Self::bind_account)). Never panics: a
    /// corrupt/unreadable file fails closed (fully spent until rollover). `today` is injected for
    /// testability.
    pub fn load(path: PathBuf, chain_id: u64, today: u64) -> Self {
        let (record, corrupt, kind) = resolve_load(std::fs::read(&path), chain_id, today);
        match &kind {
            LoadKind::Invalid(why) => eprintln!(
                "signerd: ⚠ SPEND COUNTER FALLBACK — {} exists but did not load ({why}); \
                 treating the daily cap as FULLY SPENT until the next UTC day (fail closed). \
                 Delete the file to start a fresh window.",
                path.display()
            ),
            LoadKind::WrongChain => eprintln!(
                "signerd: spend counter at {} is for a different chain — starting a fresh window",
                path.display()
            ),
            LoadKind::Missing | LoadKind::Loaded => {}
        }
        Self {
            path,
            chain_id,
            record,
            corrupt,
        }
    }

    /// Effective spend for the cap check = `committed + reserved`. A corrupt counter reads as fully
    /// spent (`U256::MAX`) so every auto-allow is refused until rollover clears it.
    pub fn effective_spent(&self) -> U256 {
        if self.corrupt {
            U256::MAX
        } else {
            self.record
                .committed_wei
                .saturating_add(self.record.reserved_wei)
        }
    }

    /// The forward-only UTC-day floor of the current window.
    pub fn day(&self) -> u64 {
        self.record.day
    }

    /// Forward-only rollover. Resets the window ONLY when the day advances; a backward wall-clock
    /// (`today <= day`) leaves the window intact — fail closed, not reset-to-zero. Returns true if
    /// it reset (the caller re-syncs the policy mirror to zero).
    pub fn rollover(&mut self, today: u64) -> bool {
        if today > self.record.day {
            self.record = SpendRecord::fresh(self.chain_id, self.record.account, today);
            self.corrupt = false; // a genuinely new day clears a corrupt-file wedge
            self.persist_best_effort();
            true
        } else {
            false
        }
    }

    /// Bind the unlocked account. If the stored account differs (a re-key, or a fresh/corrupt
    /// record whose account is the zero default), reset the window for the new account — caps are
    /// per-account. Returns true if it reset.
    ///
    /// Note this CLEARS a corrupt-file wedge: a corrupt record carries the zero-address default, so
    /// the first real unlock mismatches and starts a fresh window. That is acceptable under the
    /// conceded same-uid boundary — an attacker who can corrupt the counter file can equally delete
    /// it, which already routes to a fresh window (the accepted residual, ADR 0004). So the
    /// corrupt fail-closed (fully spent) holds only until the next unlock or rollover, not forever;
    /// it buys tamper-*evidence* (the loud log at load), not tamper-resistance.
    pub fn bind_account(&mut self, account: Address, today: u64) -> bool {
        if self.record.account != account {
            self.record = SpendRecord::fresh(self.chain_id, account, today);
            self.corrupt = false;
            self.persist_best_effort();
            true
        } else {
            false
        }
    }

    /// Reserve `value` before the signature is released. Persists durably (fsync). On a write
    /// failure the caller MUST fail closed (deny) — nothing has been signed yet.
    pub fn reserve(&mut self, value: U256) -> anyhow::Result<()> {
        self.record.reserved_wei = self.record.reserved_wei.saturating_add(value);
        self.persist()
    }

    /// Commit a reserved spend after a successful — or timed-out, which may have landed — broadcast:
    /// move it reserved → committed. Best-effort persist: the tx is already on the wire, and a
    /// persist failure leaves the reservation on disk, which still counts as spent on reboot.
    pub fn commit(&mut self, value: U256) {
        self.record.reserved_wei = self.record.reserved_wei.saturating_sub(value);
        self.record.committed_wei = self.record.committed_wei.saturating_add(value);
        self.persist_best_effort();
    }

    /// Release a reservation after a clean pre-broadcast RPC rejection (the tx did not go out).
    pub fn release(&mut self, value: U256) {
        self.record.reserved_wei = self.record.reserved_wei.saturating_sub(value);
        self.persist_best_effort();
    }

    fn persist(&self) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&self.record)?;
        atomic_write(&self.path, &bytes)
    }

    fn persist_best_effort(&self) {
        if let Err(e) = self.persist() {
            eprintln!(
                "signerd: ⚠ spend counter persist failed ({e}); accounting may revert on restart"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: u64 = 31337;
    const DAY: u64 = 20_000;

    fn acct(b: u8) -> Address {
        Address::from([b; 20])
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("deckard-spend-test-{}-{name}", std::process::id()))
    }

    // ── pure load resolver ──

    #[test]
    fn missing_file_is_a_fresh_window() {
        let read = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        let (rec, corrupt, kind) = resolve_load(read, CHAIN, DAY);
        assert_eq!(kind, LoadKind::Missing);
        assert!(!corrupt);
        assert_eq!(rec, SpendRecord::fresh(CHAIN, Address::ZERO, DAY));
    }

    #[test]
    fn unparseable_file_fails_closed_corrupt() {
        let (_rec, corrupt, kind) = resolve_load(Ok(b"{ not json".to_vec()), CHAIN, DAY);
        assert!(corrupt, "corrupt file must fail closed");
        assert!(matches!(kind, LoadKind::Invalid(_)));
    }

    #[test]
    fn unreadable_file_fails_closed_corrupt() {
        let read = Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let (_rec, corrupt, kind) = resolve_load(read, CHAIN, DAY);
        assert!(corrupt);
        assert!(matches!(kind, LoadKind::Invalid(_)));
    }

    #[test]
    fn wrong_chain_is_discarded_not_counted() {
        let other = SpendRecord {
            chain_id: 1, // mainnet record loaded by a 31337 daemon
            account: acct(7),
            day: DAY,
            committed_wei: U256::from(999u64),
            reserved_wei: U256::ZERO,
        };
        let bytes = serde_json::to_vec(&other).unwrap();
        let (rec, corrupt, kind) = resolve_load(Ok(bytes), CHAIN, DAY);
        assert_eq!(kind, LoadKind::WrongChain);
        assert!(!corrupt);
        assert_eq!(
            rec.committed_wei,
            U256::ZERO,
            "a different chain's spend never throttles this one"
        );
        assert_eq!(rec.chain_id, CHAIN);
    }

    #[test]
    fn same_chain_record_loads_verbatim() {
        let rec0 = SpendRecord {
            chain_id: CHAIN,
            account: acct(3),
            day: DAY,
            committed_wei: U256::from(5u64),
            reserved_wei: U256::from(2u64),
        };
        let bytes = serde_json::to_vec(&rec0).unwrap();
        let (rec, corrupt, kind) = resolve_load(Ok(bytes), CHAIN, DAY);
        assert_eq!(kind, LoadKind::Loaded);
        assert!(!corrupt);
        assert_eq!(rec, rec0);
    }

    // ── effective spend + conservative recovery ──

    #[test]
    fn effective_spent_counts_reserved_as_spent() {
        let path = tmp("eff");
        let _ = std::fs::remove_file(&path);
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        s.bind_account(acct(1), DAY);
        s.reserve(U256::from(10u64)).unwrap();
        assert_eq!(s.effective_spent(), U256::from(10u64));
        s.commit(U256::from(10u64));
        assert_eq!(
            s.effective_spent(),
            U256::from(10u64),
            "commit keeps the total, moves reserved→committed"
        );
        s.reserve(U256::from(4u64)).unwrap();
        s.release(U256::from(4u64));
        assert_eq!(
            s.effective_spent(),
            U256::from(10u64),
            "release rolls back a reservation"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reserved_leftover_on_reload_counts_as_spent() {
        // A crash between reserve and commit leaves reserved on disk; the reload counts it.
        let path = tmp("leftover");
        let _ = std::fs::remove_file(&path);
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        s.bind_account(acct(1), DAY);
        s.reserve(U256::from(7u64)).unwrap(); // persisted, never committed (simulated crash)
        drop(s);
        let s2 = SpendStore::load(path.clone(), CHAIN, DAY);
        assert_eq!(
            s2.effective_spent(),
            U256::from(7u64),
            "orphaned reserve is spent on reboot"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_reads_as_fully_spent() {
        let path = tmp("corrupt");
        std::fs::write(&path, b"{ truncated").unwrap();
        let s = SpendStore::load(path.clone(), CHAIN, DAY);
        assert_eq!(
            s.effective_spent(),
            U256::MAX,
            "corrupt counter = cap exhausted"
        );
        let _ = std::fs::remove_file(&path);
    }

    // ── forward-only rollover ──

    #[test]
    fn rollover_resets_only_when_day_advances() {
        let path = tmp("roll");
        let _ = std::fs::remove_file(&path);
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        s.bind_account(acct(1), DAY);
        s.reserve(U256::from(50u64)).unwrap();
        s.commit(U256::from(50u64));
        assert_eq!(s.effective_spent(), U256::from(50u64));

        assert!(!s.rollover(DAY), "same day: no reset");
        assert_eq!(s.effective_spent(), U256::from(50u64));

        assert!(
            !s.rollover(DAY - 1),
            "BACKWARD clock: must NOT reset (fail closed)"
        );
        assert_eq!(
            s.effective_spent(),
            U256::from(50u64),
            "clock rewind keeps the window"
        );

        assert!(s.rollover(DAY + 1), "forward day: resets");
        assert_eq!(s.effective_spent(), U256::ZERO);
        assert_eq!(s.day(), DAY + 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_wedge_clears_on_unlock_rebind() {
        // A corrupt file fails closed (fully spent) at load, but the first real unlock rebinds a
        // fresh window — the corrupt record's account is the zero default, so it mismatches. This
        // pins the documented behavior: the wedge holds until unlock/rollover, not forever (an
        // attacker who can corrupt the file can equally delete it — the accepted same-uid residual).
        let path = tmp("corrupt-unlock");
        std::fs::write(&path, b"}{corrupt").unwrap();
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        assert_eq!(
            s.effective_spent(),
            U256::MAX,
            "corrupt → fully spent before unlock"
        );
        assert!(
            s.bind_account(acct(1), DAY),
            "first unlock rebinds a fresh window"
        );
        assert_eq!(
            s.effective_spent(),
            U256::ZERO,
            "wedge cleared by the trusted unlock"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rollover_clears_a_corrupt_wedge_on_a_new_day() {
        let path = tmp("roll-corrupt");
        std::fs::write(&path, b"garbage").unwrap();
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        assert_eq!(s.effective_spent(), U256::MAX);
        assert!(s.rollover(DAY + 1), "a new day clears the corrupt wedge");
        assert_eq!(s.effective_spent(), U256::ZERO);
        let _ = std::fs::remove_file(&path);
    }

    // ── account binding ──

    #[test]
    fn bind_account_resets_window_on_account_change() {
        let path = tmp("bind");
        let _ = std::fs::remove_file(&path);
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        assert!(
            s.bind_account(acct(1), DAY),
            "first bind from zero-default resets"
        );
        s.reserve(U256::from(9u64)).unwrap();
        s.commit(U256::from(9u64));
        assert!(
            !s.bind_account(acct(1), DAY),
            "same account re-unlock keeps the window"
        );
        assert_eq!(
            s.effective_spent(),
            U256::from(9u64),
            "durability across re-unlock"
        );
        assert!(s.bind_account(acct(2), DAY), "different account resets");
        assert_eq!(s.effective_spent(), U256::ZERO);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reserve_persists_across_reload() {
        let path = tmp("persist");
        let _ = std::fs::remove_file(&path);
        let mut s = SpendStore::load(path.clone(), CHAIN, DAY);
        s.bind_account(acct(1), DAY);
        s.reserve(U256::from(3u64)).unwrap();
        s.commit(U256::from(3u64));
        drop(s);
        // A fresh daemon (same chain, same account, same day) recovers the committed spend.
        let mut s2 = SpendStore::load(path.clone(), CHAIN, DAY);
        assert_eq!(
            s2.effective_spent(),
            U256::from(3u64),
            "honest restart keeps the day's spend"
        );
        assert!(!s2.bind_account(acct(1), DAY));
        assert_eq!(s2.effective_spent(), U256::from(3u64));
        let _ = std::fs::remove_file(&path);
    }
}
