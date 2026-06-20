//! Reference implementation of the **keystone primitive** from
//! [`docs/adr/0004-rollback-resistant-state-anchor.md`](../../../docs/adr/0004-rollback-resistant-state-anchor.md).
//!
//! This is a SPIKE artifact: a small, **unwired** reference impl that makes the
//! `StateAnchor` interface concrete so `#72` (authenticated policy) and `#108` (durable cap)
//! can build on a real type instead of a sketch. It is gated behind the off-by-default
//! `state-anchor` feature and is wired into NO production path — `unlock`, `propose`, and
//! `execute` are untouched. Enabling the feature changes no behavior; it only compiles this
//! module and its tests.
//!
//! ## What it models (and what it deliberately does not)
//!
//! The anchor enforces two things the keystone needs and nothing more:
//! - **Monotonicity** — `advance` persists a namespace's record only if the new version is
//!   strictly greater than the stored one (a compare-and-advance), so a stale/equal write fails
//!   closed.
//! - **Durability + single-writer** — the file backend reuses the exact temp→fsync→rename→dir-sync
//!   recipe `Vault::write_atomic` already uses (`keystore.rs`), and the daemon is the sole writer
//!   (its lifetime `flock` + per-request mutex serialize every mutation).
//!
//! Integrity of a record's *payload* is the **consumer's** job, layered on top (e.g. `#72` MACs
//! `policy.json`; `#108` binds cap accounting to chain+account+policy-version+UTC-day). The anchor
//! stores opaque, monotonically-versioned bytes. And per the ADR's honest residual: the on-disk
//! backend is itself same-uid-deletable, so it raises the bar against the *weaker* attacker (a bad
//! backup, a sync glitch, a sandboxed process) and detects non-adversarial loss — it is not
//! tamper-proof against full same-uid code execution. A keychain backend (a new dependency, see the
//! ADR) is the bar-raising upgrade behind the same trait; it is not built here.

use std::path::PathBuf;

/// The artifacts that share the one anchor record, each with its own monotonic version so a vault
/// re-seal, a policy edit, and a cap-window roll advance independently (ADR 0004, Q5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    /// `#71` — the vault epoch. (No legitimate bumper exists in the shipping keystore yet; see the
    /// ADR. Present so the interface is complete, not because the vault detector is live today.)
    Vault,
    /// `#72` — the authenticated policy version, bumped on each authorized policy edit.
    Policy,
    /// `#108` — the durable daily-cap generation fence.
    Cap,
}

impl Namespace {
    fn id(self) -> u8 {
        match self {
            Namespace::Vault => 0,
            Namespace::Policy => 1,
            Namespace::Cap => 2,
        }
    }
    fn from_id(id: u8) -> anyhow::Result<Self> {
        Ok(match id {
            0 => Namespace::Vault,
            1 => Namespace::Policy,
            2 => Namespace::Cap,
            _ => anyhow::bail!("unknown anchor namespace id"),
        })
    }
}

/// One namespace's anchored value: a monotonic `version` plus an opaque, consumer-authenticated
/// `payload` (e.g. the binding `chain+account+policy_version+UTC-day` for the cap). The anchor
/// never interprets the payload; it only guarantees the version advances monotonically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorRecord {
    pub version: u64,
    pub payload: Vec<u8>,
}

impl AnchorRecord {
    pub fn new(version: u64, payload: Vec<u8>) -> Self {
        Self { version, payload }
    }
}

/// The three-valued read the ADR (Q1) requires so an unreadable anchor never bricks unlock and
/// never silently disables rollback detection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorRead {
    /// An authenticated, current record for the namespace.
    Present(AnchorRecord),
    /// First run, or a wiped domain. Indistinguishable from "an attacker deleted it" by
    /// construction — the irreducible same-uid residual (ADR 0004, Q4).
    Absent,
    /// The backend is reachable-in-principle but cannot answer right now (e.g. a locked or denied
    /// keychain). The caller proceeds on the remaining domains with a surfaced warning. The file
    /// backend never returns this — a missing file is `Absent`, a corrupt file is a hard `Err`.
    Degraded(String),
}

/// The verdict of comparing an artifact's on-disk version against the anchor, encoding the Q3
/// restore-from-backup decision table as a pure, testable function (see [`classify`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorVerdict {
    /// Anchor absent for this artifact (new machine / fresh account / wiped anchor): adopt the
    /// file's version after a successful unlock.
    Bootstrap,
    /// `file == anchor`: normal.
    Normal,
    /// `file > anchor`: a restore-forward or a legitimate advance; adopt up to the file.
    AdoptForward,
    /// `file < anchor`: rollback suspected. Gate behind a human, Control-channel confirm.
    RollbackSuspected { file: u64, anchor: u64 },
}

/// Apply the Q3 decision rule. `file_version` is read from the artifact only *after* its own
/// authentication has passed (the vault's AEAD, or a consumer's MAC); `anchor` is the
/// [`StateAnchor::read`] result for the same namespace.
pub fn classify(file_version: u64, anchor: &AnchorRead) -> AnchorVerdict {
    match anchor {
        AnchorRead::Absent | AnchorRead::Degraded(_) => AnchorVerdict::Bootstrap,
        AnchorRead::Present(rec) => {
            if file_version > rec.version {
                AnchorVerdict::AdoptForward
            } else if file_version == rec.version {
                AnchorVerdict::Normal
            } else {
                AnchorVerdict::RollbackSuspected {
                    file: file_version,
                    anchor: rec.version,
                }
            }
        }
    }
}

/// A monotonic, rollback-resistant security-state store. Implemented here by a file backend
/// (zero new dependencies); a keychain backend (a new dependency, ADR-approval-gated) would
/// satisfy the same trait. `signerd` is the only writer.
pub trait StateAnchor {
    /// The current value for `ns`, or the absent/degraded signal.
    fn read(&self, ns: Namespace) -> anyhow::Result<AnchorRead>;

    /// Monotonic compare-and-advance: persist `next` **only if** its version is strictly greater
    /// than the stored version for `ns`, *and* the stored version equals `expected` (a CAS guard
    /// against a concurrent or torn advance). Returns the committed record. Fails closed on a
    /// stale/equal version, an `expected` mismatch, or a failed durability step — never silently
    /// regresses.
    fn advance(
        &mut self,
        ns: Namespace,
        expected: u64,
        next: AnchorRecord,
    ) -> anyhow::Result<AnchorRecord>;
}

// --- File backend ---

const MAGIC: &[u8; 4] = b"DKAN"; // "DecKard ANchor" — distinct from the vault's b"DKRD"
const FORMAT_VERSION: u8 = 1;
/// Caps applied before allocating, so a hostile anchor file can't OOM us (mirrors `keystore.rs`).
const MAX_ENTRIES: u32 = 16;
const MAX_PAYLOAD_LEN: u32 = 256;
const MAX_ANCHOR_BYTES: u64 = 8 * 1024;

/// A file-backed [`StateAnchor`]: one file holds the whole namespaced record, written with the
/// atomic temp→fsync→rename→dir-sync discipline so a crash never leaves a torn anchor.
///
/// NOTE on the path: a real wiring resolves this through the same `DECKARD_CONFIG_DIR`-aware
/// resolver as the vault/policy (or a `DECKARD_ANCHOR_DIR` override), **not** raw
/// `directories::data_dir()` — otherwise the throwaway `just qa`/`just demo` vaults and the real
/// vault share one anchor namespace, and on macOS `data_dir == config_dir` anyway (ADR 0004, Q5).
/// The reference impl takes an explicit path to keep that policy out of the primitive.
pub struct FileAnchor {
    path: PathBuf,
}

impl FileAnchor {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Load the whole record set. A missing file is an empty set (every namespace `Absent`); a
    /// present-but-corrupt file is a hard error (fail closed — the caller must not proceed on a
    /// half-trusted anchor).
    fn load(&self) -> anyhow::Result<Vec<(Namespace, AnchorRecord)>> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            meta.len() <= MAX_ANCHOR_BYTES,
            "anchor file is implausibly large"
        );
        let bytes = std::fs::read(&self.path)?;
        Self::parse(&bytes)
    }

    /// Parse the on-disk format through a bounded reader (no raw indexing; every length capped
    /// before allocation), mirroring the keystore's untrusted-bytes discipline.
    fn parse(bytes: &[u8]) -> anyhow::Result<Vec<(Namespace, AnchorRecord)>> {
        let mut r = Reader::new(bytes);
        anyhow::ensure!(r.take(4)? == MAGIC, "not a Deckard anchor file");
        anyhow::ensure!(r.u8()? == FORMAT_VERSION, "unsupported anchor version");
        let count = r.u32()?;
        anyhow::ensure!(count <= MAX_ENTRIES, "too many anchor entries");
        let mut out: Vec<(Namespace, AnchorRecord)> = Vec::new();
        for _ in 0..count {
            let ns = Namespace::from_id(r.u8()?)?;
            let version = r.u64()?;
            let plen = r.u32()?;
            anyhow::ensure!(plen <= MAX_PAYLOAD_LEN, "anchor payload too large");
            let payload = r.take(plen as usize)?.to_vec();
            anyhow::ensure!(
                !out.iter().any(|(seen, _)| *seen == ns),
                "duplicate anchor namespace"
            );
            out.push((ns, AnchorRecord::new(version, payload)));
        }
        r.finish()?;
        Ok(out)
    }

    fn serialize(records: &[(Namespace, AnchorRecord)]) -> Vec<u8> {
        let mut b = Vec::with_capacity(64);
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (ns, rec) in records {
            b.push(ns.id());
            b.extend_from_slice(&rec.version.to_le_bytes());
            b.extend_from_slice(&(rec.payload.len() as u32).to_le_bytes());
            b.extend_from_slice(&rec.payload);
        }
        b
    }

    /// Atomic write: temp file at `0600`, `write_all`, `sync_all`, `rename` over the target, then
    /// `fsync` the parent dir — the same recipe as `Vault::write_atomic` (`keystore.rs`), so a
    /// crash or power loss never leaves a partially written anchor.
    fn write_atomic(&self, records: &[(Namespace, AnchorRecord)]) -> anyhow::Result<()> {
        use std::io::Write;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(&Self::serialize(records))?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        if let Some(dir) = self.path.parent() {
            if let Ok(dirf) = std::fs::File::open(dir) {
                let _ = dirf.sync_all();
            }
        }
        Ok(())
    }
}

impl StateAnchor for FileAnchor {
    fn read(&self, ns: Namespace) -> anyhow::Result<AnchorRead> {
        let records = self.load()?;
        Ok(match records.into_iter().find(|(n, _)| *n == ns) {
            Some((_, rec)) => AnchorRead::Present(rec),
            None => AnchorRead::Absent,
        })
    }

    fn advance(
        &mut self,
        ns: Namespace,
        expected: u64,
        next: AnchorRecord,
    ) -> anyhow::Result<AnchorRecord> {
        let mut records = self.load()?;
        let current = records
            .iter()
            .find(|(n, _)| *n == ns)
            .map(|(_, r)| r.version);
        match current {
            Some(v) => {
                anyhow::ensure!(
                    v == expected,
                    "anchor advance conflict: expected version {expected}, found {v}"
                );
                anyhow::ensure!(
                    next.version > v,
                    "anchor advance must be monotonic: {} is not greater than {v}",
                    next.version
                );
            }
            None => {
                // Bootstrap: the caller must claim it expected no prior entry.
                anyhow::ensure!(
                    expected == 0,
                    "anchor bootstrap expects version 0, got {expected}"
                );
            }
        }
        match records.iter_mut().find(|(n, _)| *n == ns) {
            Some((_, slot)) => *slot = next.clone(),
            None => records.push((ns, next.clone())),
        }
        self.write_atomic(&records)?;
        Ok(next)
    }
}

/// A tiny bounds-checked reader for the anchor format (the keystore's `Reader` is private to that
/// module; this mirrors it so untrusted anchor bytes are parsed with the same discipline).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.buf.len())
            .ok_or_else(|| anyhow::anyhow!("anchor truncated"))?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| anyhow::anyhow!("anchor truncated"))?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> anyhow::Result<u8> {
        self.take(1)?
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("anchor truncated"))
    }
    fn u32(&mut self) -> anyhow::Result<u32> {
        let b: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("anchor truncated"))?;
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> anyhow::Result<u64> {
        let b: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("anchor truncated"))?;
        Ok(u64::from_le_bytes(b))
    }
    fn finish(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.pos == self.buf.len(), "trailing bytes after anchor");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        // A unique-enough path per test; OsRng would be overkill for a temp file name.
        std::env::temp_dir().join(format!(
            "deckard-anchor-test-{tag}-{}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn bootstrap_then_monotonic_advance() {
        let path = temp_path("mono");
        let _ = std::fs::remove_file(&path);
        let mut a = FileAnchor::at(&path);

        // Fresh: every namespace reads Absent.
        assert_eq!(a.read(Namespace::Policy).unwrap(), AnchorRead::Absent);

        // Bootstrap at 1, then advance 1->2->3.
        a.advance(Namespace::Policy, 0, AnchorRecord::new(1, vec![]))
            .unwrap();
        a.advance(Namespace::Policy, 1, AnchorRecord::new(2, b"v2".to_vec()))
            .unwrap();
        a.advance(Namespace::Policy, 2, AnchorRecord::new(3, b"v3".to_vec()))
            .unwrap();

        match a.read(Namespace::Policy).unwrap() {
            AnchorRead::Present(rec) => {
                assert_eq!(rec.version, 3);
                assert_eq!(rec.payload, b"v3");
            }
            other => panic!("expected Present, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_or_equal_advance_fails_closed() {
        let path = temp_path("stale");
        let _ = std::fs::remove_file(&path);
        let mut a = FileAnchor::at(&path);
        a.advance(Namespace::Cap, 0, AnchorRecord::new(5, vec![]))
            .unwrap();

        // Equal version is not strictly greater -> rejected.
        assert!(a
            .advance(Namespace::Cap, 5, AnchorRecord::new(5, vec![]))
            .is_err());
        // A lower version (an attempted rollback write) -> rejected.
        assert!(a
            .advance(Namespace::Cap, 5, AnchorRecord::new(4, vec![]))
            .is_err());
        // A stale `expected` (CAS conflict) -> rejected even though 9 > 5.
        assert!(a
            .advance(Namespace::Cap, 4, AnchorRecord::new(9, vec![]))
            .is_err());

        // The stored value is unchanged after every rejected write (no silent regression).
        assert_eq!(
            a.read(Namespace::Cap).unwrap(),
            AnchorRead::Present(AnchorRecord::new(5, vec![]))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn namespaces_advance_independently() {
        let path = temp_path("ns");
        let _ = std::fs::remove_file(&path);
        let mut a = FileAnchor::at(&path);
        a.advance(Namespace::Vault, 0, AnchorRecord::new(1, vec![]))
            .unwrap();
        a.advance(Namespace::Policy, 0, AnchorRecord::new(7, vec![]))
            .unwrap();
        a.advance(Namespace::Cap, 0, AnchorRecord::new(42, vec![]))
            .unwrap();
        // Advancing one leaves the others untouched.
        a.advance(Namespace::Policy, 7, AnchorRecord::new(8, vec![]))
            .unwrap();
        assert_eq!(
            a.read(Namespace::Vault).unwrap(),
            AnchorRead::Present(AnchorRecord::new(1, vec![]))
        );
        assert_eq!(
            a.read(Namespace::Cap).unwrap(),
            AnchorRead::Present(AnchorRecord::new(42, vec![]))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persists_across_reopen() {
        let path = temp_path("reopen");
        let _ = std::fs::remove_file(&path);
        {
            let mut a = FileAnchor::at(&path);
            a.advance(
                Namespace::Policy,
                0,
                AnchorRecord::new(3, b"state".to_vec()),
            )
            .unwrap();
        }
        // A fresh handle (a daemon restart) sees the durably-written record.
        let b = FileAnchor::at(&path);
        assert_eq!(
            b.read(Namespace::Policy).unwrap(),
            AnchorRead::Present(AnchorRecord::new(3, b"state".to_vec()))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_fails_closed_not_absent() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"not a deckard anchor at all").unwrap();
        let a = FileAnchor::at(&path);
        // A corrupt anchor must be a hard error (caller fail-closes), never silently treated as
        // Absent (which would route into bootstrap and accept whatever the file claims).
        assert!(a.read(Namespace::Policy).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_through_bounded_reader() {
        let records = vec![
            (Namespace::Vault, AnchorRecord::new(1, vec![])),
            (Namespace::Policy, AnchorRecord::new(9, b"abc".to_vec())),
            (Namespace::Cap, AnchorRecord::new(u64::MAX, vec![0xFF; 16])),
        ];
        let bytes = FileAnchor::serialize(&records);
        let parsed = FileAnchor::parse(&bytes).unwrap();
        assert_eq!(parsed, records);
        // Trailing garbage is rejected.
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(FileAnchor::parse(&extra).is_err());
        // A truncated buffer is rejected, not silently short-read.
        assert!(FileAnchor::parse(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn classify_encodes_the_restore_decision_table() {
        // Absent -> bootstrap (new machine / wiped anchor).
        assert_eq!(classify(5, &AnchorRead::Absent), AnchorVerdict::Bootstrap);
        // Degraded -> bootstrap (proceed; the keychain is unreachable).
        assert_eq!(
            classify(5, &AnchorRead::Degraded("locked".into())),
            AnchorVerdict::Bootstrap
        );
        let anchor = AnchorRead::Present(AnchorRecord::new(7, vec![]));
        // file == anchor -> normal.
        assert_eq!(classify(7, &anchor), AnchorVerdict::Normal);
        // file > anchor -> adopt forward (restore-forward / legitimate advance).
        assert_eq!(classify(8, &anchor), AnchorVerdict::AdoptForward);
        // file < anchor -> rollback suspected (the only branch the human Control-gate guards).
        assert_eq!(
            classify(6, &anchor),
            AnchorVerdict::RollbackSuspected { file: 6, anchor: 7 }
        );
    }
}
