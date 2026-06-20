# ADR 0004 — Rollback-resistant security-state anchor (the keystone)

- **Status:** Proposed (2026-06-20). Records the design for issue `#71` and the primitive that
  `#72` (authenticated policy) and `#108` (durable cap) build on. Decisions only; executable work
  stays in GitHub issues.
- **Deciders:** @hellno (maintainer)
- **Method:** source-grounded review (every load-bearing fact below is cited `file:line` and was
  grepped on a fresh worktree off `origin/main` post-`#105`), an **empirical** dependency-cost
  measurement (`cargo tree` on this macOS host, diffed against the 1145-crate workspace lock), and a
  fan-out research + **adversarial verification** pass (four research strands, then independent
  skeptics tasked to refute the consolidated design; 16 of their attacks landed and reshaped the
  conclusion below). A planned codex cross-model pass did not run (session limit); it is the one gap
  in the method and is noted as a follow-up.
- **Context inputs:** [`ADR 0003`](0003-crate-trust-boundary.md) (the keystone framing, items #4–#7),
  `THREAT-MODEL.md`, `SECURITY.md`, `crates/deckard-core/src/keystore.rs`,
  `crates/deckard-core/src/config.rs`, `crates/deckard-signerd/src/{daemon,config,policy_store}.rs`,
  the dependent issues [`#71`](https://github.com/hellno/deckard/issues/71),
  [`#72`](https://github.com/hellno/deckard/issues/72),
  [`#108`](https://github.com/hellno/deckard/issues/108).

## The question

`vault.bin` is AEAD-encrypted, so a same-uid attacker with filesystem write can't *forge* a new valid
vault. But they can *roll it back*: drop an older, genuine copy of the user's own vault over the
current one (malware running as you, a careless restore, a sync conflict). The stale vault opens
cleanly under the right passphrase and resurrects old state. Nothing detects this today (zero
OS-keychain use in `deckard-core` / `deckard-signerd`).

ADR 0003 elevated this from a vault-only fix to the **keystone**: the same mechanism a vault needs to
detect rollback (a monotonic counter bumped on every authoritative write, mirrored to a reference in a
different trust domain) is what authenticated policy (`#72`) and a durable daily cap (`#108`) also
need. So this ADR answers `#71`'s five spike questions and designs the shared primitive those two
consumers stand on.

## The headline, stated before the details

The adversarial pass changed the answer. Three things are true and must be said plainly:

1. **The anchor crate is cheap, and the recommendation is the OS keychain on macOS/Windows plus a
   file fallback everywhere — but it is a new dependency that needs explicit approval, and on Linux it
   is mostly absent where it is most needed.** We measured the cost; it is small. We do **not** add it
   in this PR.

2. **The two correctness details from the issue are real and we have the exact mechanism for both.**
   Binding the epoch into the AEAD associated data is the *only* way the file itself carries a
   trustworthy epoch, and it is correct. Checking the anchor only after the passphrase verifies is a
   one-line ordering decision at the unlock seam.

3. **The standalone "anchor the vault" feature, taken literally, is close to theater on today's
   codebase, and the honest residual is starker than "raises the bar."** Nothing legitimately
   advances a vault's epoch (every seal mints a fresh identity; there is no re-seal path). A
   plaintext sidecar epoch is rolled back together with the vault and verifies fine. The anchor itself
   is same-uid-deletable, and deleting it routes straight into the "new machine, bootstrap" path with
   no challenge. So the keystone earns its keep first on **policy and cap** (artifacts that *do* have
   legitimate monotonic bumps), not on the vault. This ADR is conclusive, and the conclusion is to
   **build the generalizable primitive, wire it to `#72`/`#108` first, and defer the vault-epoch
   binding to a v2-format effort that also builds the missing re-seal path.**

The rest of this document is the evidence for those three claims, mapped onto the five spike questions.

---

## Q1 — Anchor crate and the real new-dependency cost

**Decision: recommend `keyring` (pinned, `default-features = false`, per-OS native backend) as the
durable trust-domain reference on macOS and Windows; ship file-only on Linux; and ship a
dependency-free file backend as the always-present baseline on every OS. This is a recommendation that
needs explicit maintainer sign-off against the no-new-deps DoD bar. It is not added in this PR.**

### What we measured (this macOS host, diffed against `Cargo.lock`)

| Config | Activated tree | **Crates not already in the workspace lock** |
|---|---|---|
| `keyring` 3.6.3, `apple-native` (macOS) | 8 | **1** — `keyring` itself |
| `keyring` 3.6.3, `sync-secret-service` + `vendored` + `crypto-rust` (Linux) | 38 | **4** — `keyring`, `dbus`, `dbus-secret-service`, `libdbus-sys` (a **C** library) |

On macOS the cost is one Rust crate. `security-framework` (which is also the Touch ID hook the issue
flagged for a later phase), `core-foundation`, `bitflags`, `libc`, and `log` are already in the lock
via alloy/helios, so `apple-native` reuses them. `keyring` is MIT/Apache (compatible with our
AGPL-3.0), MSRV 1.75, and exposes a process-wide **mock** credential store usable as the test backend
and the "no keychain present" shim.

On Linux the cost is qualitatively larger than four crates suggests:

- The D-Bus Secret Service backend pulls a **C library** (`libdbus-sys`) and, at runtime, needs a
  live session bus **and** an unlocked keyring daemon (gnome-keyring / KWallet). A headless server
  has neither, and `signerd` is exactly the kind of process that runs headless. The call fails to
  find an item rather than returning a stable anchor.
- The kernel `keyutils` backend (`linux-native`) avoids D-Bus entirely but is non-persistent across
  reboot by default. An anchor must survive reboot, so `keyutils` is unsuitable as the primary store.

So on Linux the honest answer is **file-only**, with the keychain as a best-effort extra only where a
provider happens to be present.

### What we ask the maintainer to approve (and what we do not)

The real ask is small: **`keyring` on macOS and Windows only.** Pin it (`= 3.6.3` or a tilde range,
not the caret `"3"`, which is an open range whose transitive closure can drift and which the DoD bar
would not actually freeze), and re-measure the closure on the unified workspace before adding it.
Per ADR 0003 item #8, confirm via `cargo tree` on the real workspace that `keyring`/`libdbus` stay
**out of `deckard-app`'s** feature closure (Cargo feature unification can silently re-add a backend);
the anchor and its FFI belong to `signerd`, the single key-holder, never to `deckard-core`
(`#![forbid(unsafe_code)]`, and linked by every key-less binary). `keyring` 4.1.1 exists and is a
major bump (the `keyring-core` + store split); evaluate it separately, do not adopt blind.

Rejected alternatives: raw `SecItem` / `libsecret` / `wincred` FFI (more `unsafe` to own and audit for
a control that is not load-bearing against same-uid); `keyring-core` + a store (premature at our
scale). The file backend ships with **zero new dependencies** and is the baseline; the keychain is the
bar-raising upgrade layered behind the same trait, not a prerequisite.

### Degraded mode is mandatory, and it is the honest core of the feature

An anchor that cannot be read must never brick unlock and must never silently disable rollback
detection. The read path is three-valued: `Present` (an authenticated record), `Absent` (first run, or
a wiped domain), `Degraded` (the backend is unreachable, e.g. a locked or denied keychain). A detected
**regression** (a present domain reporting a lower version than expected) fails closed; a merely
**absent** domain degrades to the remaining domains with a surfaced warning, in the spirit of the
`⚠ POLICY FALLBACK` line `policy_store.rs` already prints. The catch, which Q4 makes precise, is that
`Absent` and "an attacker deleted it" are indistinguishable by construction.

---

## Q2 — The two correctness details at the keystore seam

### (a) Bind the epoch into the AEAD associated data

`keystore.rs` already authenticates the entire header through `Header::core_bytes()` (`keystore.rs:146`),
which both AEAD layers consume: `wrap_aad = [AAD_WRAP, &core]` (`keystore.rs:262`) and
`payload_aad = [AAD_PAYLOAD, &core, &wrapped_dek]` (`keystore.rs:269`). Anything inside `core` is
covered by **both** Poly1305 tags. So adding a `u64` epoch field to `Header` and emitting it in
`core_bytes()` makes it un-editable without the KEK:

> An attacker copies an old blob (epoch 3) over the current file (epoch 7), then edits the plaintext
> epoch bytes 3→7 to satisfy the anchor. The parsed `core` now says 7, but the stored tag was computed
> over 3, so `aead_decrypt` of the wrapped DEK returns `Err` and unlock fails closed — identical in
> mechanism to the existing `m_kib` tamper case in `tamper_each_region_fails_closed`
> (`keystore.rs:822-835`). The epoch becomes editable only by someone who holds the passphrase.

This is the **only** construction that lets the *file itself* carry a trustworthy epoch. A plaintext
sidecar epoch, even one MAC'd by a vault-derived key, does **not** achieve it: a same-uid attacker
rolls the sidecar back alongside the vault, both records verify against their own (older, genuine)
tags, and no forgery is needed (adversarial finding, critical). With a sidecar, *all* rollback
resistance reduces to the external anchor's high-water mark, which is same-uid-deletable. This is the
fork in the design, and the ADR resolves it explicitly below.

**Decision: AAD-binding is the correct mechanism, and it requires a format migration we do NOT land in
this spike.** `FORMAT_VERSION` is the first authenticated byte in `core_bytes()` (`keystore.rs:149`),
and the three frozen KAT fixtures (`decode_compat_v1_fixtures`, `keystore.rs:788`) are exact byte
blobs whose tags were computed over a 101-byte core with no epoch field. Inserting the field bumps the
version to 2, shifts every later offset, and breaks the fixtures (`keystore.rs:789` calls this a
lost-funds-class break). The deliverable here is an ADR, not a format migration. We therefore:

- specify the v2 layout (epoch as a `u64` LE field in `core_bytes()`, a `Reader::u64()` helper
  mirroring the existing `u32()` at `keystore.rs:639`, version dispatch in `from_bytes`),
- record that a real v2 reads v1 vaults as **epoch 0 implicit**, reconstructing the exact v1 `core` so
  the frozen tags still verify, and
- **forbid format-downgrade re-anchoring** in that future v2 work: once a `vault_id` has a v2 anchor
  entry ≥ 1, presenting the original v1 (epoch 0) blob is a rollback, never a benign bootstrap (an
  adversarial finding: otherwise a human who clicks through the restore prompt re-anchors down to 0 and
  permanently disarms detection).

The spike proves the binding with an isolated test on a v2-shaped header; the production binding is a
separate, in-scope-later format break tracked on `#71`.

### (b) Check the anchor only after the passphrase verifies

The compare lands in `signerd`, in `daemon.rs unlock()` (the success arm after the `spawn_blocking`
unlock returns `Ok(Ok(unlocked))` at `daemon.rs:421`), **not** inside core's `Vault::unlock`. Core
stays format-only, `#![forbid(unsafe_code)]`, and dependency-free; the out-of-file anchor is a
platform concern for the single key-holder.

Because the compare runs only on the branch where the AEAD already proved the passphrase, it adds no
oracle to an **unauthenticated** caller: a wrong passphrase still collapses to `BadPassphrase` through
the keystore's one-generic-message contract (`keystore.rs:419-436`), unchanged. Two qualifications the
adversarial pass forced, which the implementation and any copy must respect:

- **The reader for the file epoch must run after AEAD success, never as a pre-check.** A pre-check that
  touched a missing or garbage epoch source before the passphrase is verified would re-introduce a
  wallet-presence oracle.
- **`Unlock` is served on the public proposer socket** (only `Resolve` is `Channel::Control`-gated,
  `daemon.rs:320`). So a distinct "rolled back" outcome, if we add one, is visible to any same-uid
  proposer that already knows the passphrase. That is acceptable inside the uid boundary (such a caller
  has already unlocked) but it is **not** resolver-only, so the claim is "no oracle to an
  unauthenticated caller," not "no oracle." (A pre-existing presence oracle also remains: `unlock()`
  returns `NoVault` at `daemon.rs:409` before any passphrase check. The anchor work does not add to it
  and does not fix it.)

### Key the anchor on a re-seal-stable identity, not the per-seal `vault_id`

`seal()` mints a fresh random `vault_id` on every seal (`keystore.rs:242`). Keying the anchor on
`vault_id` means a genuine older backup of the *same seed* carries a *different* `vault_id`, lands in
the "absent → bootstrap" branch, and is silently accepted (adversarial finding). Key instead on a
domain-separated commitment to the **stable primary address**
(`HMAC(domain_key, primary_address)`), which survives re-seals; the seed never leaves core. This also
exposes the next finding: with `vault_id` keying, the vault epoch is effectively write-once.

---

## Q3 — The legitimate restore-from-backup accept path

The decision rule, keyed on the stable identity and run only after AEAD success:

| Observed | Meaning | Action |
|---|---|---|
| anchor file absent, or identity absent from it | new machine / fresh account / wiped anchor | **bootstrap**: adopt the file's value after a successful unlock |
| `file > anchor` | restore-forward, or a legitimate advance | **adopt** up to `file` |
| `file == anchor` | normal | proceed |
| `file < anchor` (identity present) | **rollback suspected** | **gate**: a one-time, human-confirmed "this vault is older than this machine last saw — restore anyway?" |

The confirm rides the existing `Channel::Control` resolver capability (the same socketpair fd that
authorizes `Resolve`, `daemon.rs:312-327`), so an injected agent on the public socket cannot
auto-confirm. On confirm we re-anchor down to the file's value, so the restored backup becomes the new
baseline and the next unlock is normal.

**Default posture for the testnet-only alpha: fail-open-with-confirm, not fail-closed-refuse.** The
dominant real-world event is a benign restore or sync glitch, not an attacker; refusing would brick
legitimate restores and teach users to disable the check. The posture is a documented dial: when
Deckard moves toward real funds, the same machinery flips to fail-closed (refuse unless a Control
confirm is present) without redesign. Anchors are **machine-local and never synced**; syncing one
would let a rollback on one device authorize itself on another. A multi-device user who restores an
older backup sees one confirm per device (and, with actively-synced state, possibly one confirm per
out-of-order sync event, which the UX must expect rather than treat as a bug).

### Two hard problems the rule alone does not solve, decided here rather than deferred

**The confirm gate is a safety feature for benign restores, not a security control.** The dominant
attacker move is not to downgrade past a surviving anchor (the only branch the gate guards). It is to
**delete the anchor** (same-uid filesystem write is in scope) and present the old vault, which routes
to "absent → bootstrap" and is accepted silently with no challenge. Deletion is indistinguishable from
a new machine by construction. The keychain copy, where present, is the only thing that makes deletion
noisier than overwriting the vault; on file-only Linux there is no such thing. We state this in the
copy and rank it as the feature's #1 residual, rather than describing the gate as making rollback
unforgeable.

**The torn-write order must be a recoverable journal, not a brick.** The anchor and the file are
separate stores, so a bump can never be one atomic transaction. Writing the anchor first is fail-closed
but bricks the wallet on any benign crash between the two writes (anchor at N, file still at N-1, read
as a rollback of a vault that was never rolled back). And the obvious un-brick ("if `file == anchor-1`
and the tags verify, auto-repair") is itself a one-step-rollback laundering primitive, because a
crash and a deliberate one-epoch rollback are indistinguishable at that point. The decision: write a
small **intent record** `{identity, old, new}` to the anchor domain, then the file, then clear the
intent. On boot, a pending intent whose `new == anchor` and `file == old` is a *provable* torn write
(advance and clear); `file < old` is a rollback (gate). This removes the ambiguity instead of guessing.
For the alpha, the simpler fallback is acceptable: treat `file == anchor` as the only steady state and
require an explicit Control-channel repair for anything else, paying the UX cost honestly.

---

## Q4 — The honest residual

`THREAT-MODEL.md`'s boundary is the uid, including filesystem write. The anchor lives inside that
boundary, so it is **resistance, not prevention**, and the honesty has to be stated *per configuration*
because the bar moves by a very different amount in each:

- **Keychain present (macOS / Windows, interactive session):** meaningfully noisier. To replay an old
  state the attacker must delete or rewrite a Keychain / Credential Manager item in a separate trust
  domain, not just overwrite a file. This is the configuration that earns the "raises the bar" claim,
  and it is the path to a future hardware-backed anchor (Secure Enclave / TPM).
- **File-only (the dependency-free default, and the *only* option on headless Linux, which the
  dep-cost analysis shows is exactly where `signerd` most often runs):** marginal. The anchor and any
  fast counter file are plain same-uid files. A full same-uid code-execution attacker deletes both and
  drops to the bootstrap path. The bar moves from "silently edit one number" to "delete two files and
  trigger a fresh-machine bootstrap." That is real against the **weaker** attacker the feature is
  honestly for (a bad backup, a sync conflict, a careless restore, a sandboxed or limited process that
  can read but not freely delete), and it is **zero** against full same-uid code execution.

So the precise claim is: the anchor detects and raises the cost of **replay of an older genuine state**
by the weaker attacker, and on keychain-backed platforms it forces that replay into a second,
hardware-backable trust domain. It does **not** stop a same-uid attacker who can delete every anchor
copy, and on file-only platforms that reduces to non-adversarial protection. A residual row is added to
`THREAT-MODEL.md` so the headless `signerd` case is never silently credited with the keychain-grade
increment it does not get.

---

## Q5 — Generalization: the keystone primitive for `#72` and `#108`

**Decision: one `StateAnchor` over a single keyed record with per-artifact *namespaced* monotonic
fields, not a single global counter.** Vault re-seal, policy edit, and cap reservation advance at
different rates and for different reasons; a shared counter would couple them (a vault re-seal would
invalidate the cap window; a policy edit would have to re-stamp cap state). Domain separation lets each
advance independently while one write commits them atomically.

```
struct AnchoredState {        // serialized + integrity-tagged as one blob
    format: u8,               // the anchor's own format version, independent of vault.bin
    vault_epoch: u64,         // #71: bumps on re-seal / DEK rotation (no legitimate bumper exists YET)
    policy_version: u64,      // #72: bumps on each authorized policy edit
    cap_generation: u64,      // #108 fence: bumps on UTC-day roll / policy change / detected rollback
    last_seen_day: u64,       // #108: monotonic max-day-ever-seen, so a backward clock can't reset
}
// AAD domain separation per field, e.g. b"DKRDv1/anchor/{vault,policy,cap}"
```

```rust
/// A monotonic, rollback-resistant security-state store. Implemented by a keychain backend
/// (a NEW DEP, needs approval) and a file/mock backend (zero new deps). signerd is the only writer.
pub trait StateAnchor {
    /// Current value for `ns`, or the degraded/absent signal.
    fn read(&self, ns: Namespace) -> anyhow::Result<AnchorRead>;
    /// Monotonic compare-and-advance: persist `next` only if its version is strictly greater
    /// than the stored version for `ns`. Fail closed on a stale/equal version, a failed
    /// durability step, or a torn/absent backend.
    fn advance(&mut self, ns: Namespace, expected: u64, next: AnchorRecord) -> anyhow::Result<AnchorRecord>;
}
enum AnchorRead { Present(AnchorRecord), Absent, Degraded(String) }
```

**Durability and the single writer.** The on-disk backend reuses the exact recipe `Vault::write_atomic`
already implements (`keystore.rs:378-405`): open the temp file at `0600`, `write_all`, `sync_all`,
`rename` over the target, then `fsync` the parent directory. `signerd` is the sole writer: it holds the
single-instance `flock` for its lifetime and its per-request mutex serializes every mutation, so no
second writer can race. The anchor path resolves through the **same `DECKARD_CONFIG_DIR`-aware
resolver** as the vault and policy (or a parallel `DECKARD_ANCHOR_DIR`), **not** raw
`directories::data_dir()` — otherwise the throwaway `just qa` / `just demo` vaults and the real vault
share one anchor namespace, and on macOS `data_dir == config_dir` anyway, so "survives a config wipe"
is a Linux-only and largely illusory benefit.

**Integrity is the consumer's job, layered on top.** The anchor enforces monotonicity and durability;
each consumer authenticates its own payload. This sidesteps an unresolved keying question (the
vault-derived MAC key is only available after unlock, which is fine for policy/cap checks that run only
while unlocked, but a policy edit authorized while locked has no key to re-MAC; that consumer must
require an unlocked session or a separate bootstrap key). Authentication reuses the existing
XChaCha20-Poly1305 (`chacha20poly1305` is an unconditional `deckard-core` dep; `hmac`/`sha2` are
`shield`-gated), so the keyed tag is zero new dependencies and the same audited family as the keystore.

### How each consumer uses it, and where each consumer is honestly weakest

- **`#72` (authenticated policy).** Today `contract::Policy` (`policy.rs:17-36`) has **no version field
  and no MAC**, and `policy.json` is plain `serde_json` (`policy_store.rs:61`) — that *is* finding C2.
  So `#72` must first add a versioned, MAC'd policy record (`version + tag`, with `version = 0` as the
  pre-versioning default for forward-compat). It then `read(Policy)`, fails closed on a bad/missing tag
  or a stale version (replay of an older, more permissive policy), and on an authorized edit calls
  `advance(Policy, …)` **before** re-MAC'ing the file. This is the keystone's strongest consumer: a
  policy version *does* advance on every legitimate edit, so `file < anchor` is reachable through real
  use, unlike the vault.

- **`#108` (durable cap).** Two tiers, because the cap increments on every spend and a keychain write
  per spend is slow and can prompt. The slow anchor holds a coarse `cap_generation` bound to
  `chain + account + policy_version + UTC-day`; the per-spend `reserved_wei` lives in a fast local file
  written with the atomic recipe, **reserved before the signature is released** (closing the
  post-broadcast crash window at `daemon.rs:1166`). Three corrections the adversarial pass forced into
  the design, none optional:
  1. **The generation fence does not catch an intra-generation rollback.** Within a UTC day at fixed
     policy, `cap_generation` is constant, so swapping in an earlier same-day fast file (lower
     `reserved_wei`, same generation) passes the fence and resets the counter. `#108` must add
     intra-generation monotonic protection (a high-water sequence the slow tier checkpoints), or scope
     the fence honestly to cross-generation only and accept that an intra-day same-uid file swap is not
     closed. "Rolling the generation retires every older snapshot" is false for same-generation
     snapshots.
  2. **A backward wall-clock jump must not re-open the window.** `current_utc_day()`
     (`policy_store.rs:98`) is naive and `rollover()` (`daemon.rs:1397`) resets the spend bidirectionally
     today. Derive the window from `effective_day = max(current_utc_day(), anchor.last_seen_day)` and
     make `rollover()` forward-only. A backward jump then mismatches and fails closed instead of
     resetting the cap to zero.
  3. **Reconcile a pre-broadcast crash deterministically.** Reserve-before-sign over-counts if the
     daemon crashes after reserving but before broadcasting. The reservation must carry a deterministic
     tx identity (`chain + from + nonce`) so that on reboot the daemon queries the chain and commits or
     releases the reservation, rather than permanently consuming the cap. Note this adds an `fsync` to
     every signature under the mutex held across broadcast, which lengthens STOP latency; measure it.

- **`#71` (the vault, the nominal first consumer, and the weakest).** There is **no legitimate bumper
  for `vault_epoch`** in the shipping code: `seal()` always mints a fresh identity, and the
  "upgrade-on-unlock re-seal" the design would lean on (`keystore.rs:60-64`) is a doc comment, not
  code. So for any one vault the epoch is write-once and `file < anchor` is unreachable through
  legitimate use; the standalone vault-rollback detector is vacuous until a re-seal-preserving-identity
  path exists. That path is itself the v2-format change Q2 defers. This is why the build order below
  puts the vault last.

---

## Decision and build order

1. **Build the generalizable `StateAnchor` primitive first** (file-backed, zero new deps), as the
   keystone ADR 0003 item #4 asks for: one namespaced authenticated record, atomic write + `fsync` +
   dir-sync, single-writer via the existing `flock`, three-valued read with fail-closed-on-regression
   and degraded-on-absent, path resolved through the config-dir-aware resolver. A small,
   feature-gated, unwired reference implementation ships **with this ADR** to make the interface
   concrete and de-risk the consumers (it changes no production behavior).
2. **Wire `#72` and `#108` to it** — they are where a monotonic counter actually advances and where
   the keystone earns its keep. `#72` must add the versioned MAC'd policy record first; `#108` must add
   the intra-generation guard, the monotonic-day guard, and pre-broadcast reconciliation.
3. **Defer the vault-epoch binding** to a v2-format effort on `#71` that also builds the missing
   re-seal-preserving-identity path and forbids format-downgrade re-anchoring. Until then, the vault
   nominally consumes the primitive at a fixed bootstrap epoch and gains the restore-confirm UX, but
   the real anti-rollback value for the vault waits on v2.
4. **The keychain backend (`keyring`) is a recommended, approval-gated upgrade**, not landed here. Ask:
   `keyring` pinned, macOS/Windows native only, file-only on Linux, closure re-measured on the unified
   workspace.

## Consequences

- **Positive:** the keystone is specified as one primitive three consumers share, with the durability
  and single-writer semantics ADR 0003 demanded; the dependency cost is measured, not guessed, and the
  approval ask is small and honest; the design is corrected for the rollback-bypass, oracle,
  torn-write, clock-rollback, intra-generation, and env-isolation holes an adversarial pass found
  before any code shipped.
- **Cost:** the strongest version of the feature needs a new dependency (approval-gated) and, for the
  vault, a v2 format migration plus a re-seal path that does not exist yet. The cap's reserve-before-
  sign adds per-spend `fsync` under the broadcast-held mutex.
- **Deferred:** the vault-epoch AAD binding (v2 format + re-seal), the hardware-backed anchor (Secure
  Enclave / TPM), Touch ID unlock (separate effort; note that `apple-native` already pulls
  `security-framework`, so this does not foreclose it), and the codex cross-model review pass that did
  not run.

## Status / next step

**Proposed.** The spike is conclusive: the anchor crate and its cost are chosen, both correctness
details have an exact mechanism, the restore path and the per-configuration residual are defined, and
the generalization is specified with its consumers' weakest points named. The decision is to build the
shared primitive and wire policy/cap first, defer the vault binding to a v2 effort, and seek approval
for the `keyring` upgrade. `#71` stays **open** (the vault binding is not done); this ADR is referenced
from it. Refinement comments on `#72` and `#108` point them at the interface above. Promote to Accepted
once the primitive lands and the first consumer (`#72`) is wired.
