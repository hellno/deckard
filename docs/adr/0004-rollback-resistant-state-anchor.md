# ADR 0004 — Vault rollback resistance: evaluated, deferred

- **Status:** **Deferred** (2026-06-20). Records the result of the `#71` spike: we evaluated
  rollback resistance for `vault.bin` and decided **not to build it now**, with concrete revisit
  conditions. It also corrects the framing that bundled `#71`, `#72`, and `#108` together.
- **Deciders:** @hellno (maintainer)
- **Method:** source-grounded review (cited `file:line`), an empirical dependency-cost measurement
  (`cargo tree` on macOS, diffed against the workspace lock), a fan-out adversarial review (16
  attacks landed), and an independent cross-model adjudication (Codex). All four pointed the same way.
- **Context inputs:** [`ADR 0003`](0003-crate-trust-boundary.md), `THREAT-MODEL.md`, `SECURITY.md`,
  `crates/deckard-core/src/keystore.rs`, `crates/deckard-signerd/src/{daemon,policy_store}.rs`,
  issues [`#71`](https://github.com/hellno/deckard/issues/71),
  [`#72`](https://github.com/hellno/deckard/issues/72),
  [`#108`](https://github.com/hellno/deckard/issues/108).

## The question

`vault.bin` is AEAD-encrypted, so it can't be forged, but a same-uid attacker with filesystem write
can replace it with an older, genuine copy (a rollback / replay). `#71` asked whether to detect that
with a monotonic counter bumped on every save and mirrored to a reference in a different trust domain
(an OS keychain, or a TPM), checked on unlock.

ADR 0003 framed `#71` as a shared foundation that `#72` (authenticate `policy.json`) and `#108` (make
the daily cap durable) would also build on. **That framing was the mistake**, and this ADR corrects it.

## Decision

1. **Do not build vault rollback detection now.** Defer `#71` with the blockers below.
2. **Do not build a shared cross-trust-domain anchor at all** (no OS-keychain mirror, no TPM counter,
   no new on-disk vault format). It was one expensive mechanism invented to serve three issues, and
   only the weakest of the three ever needed it.
3. **`#72` and `#108` are independent local fixes** that do not depend on `#71` and do not need an
   anchor. They proceed on their own. See "What we do instead."
4. This **supersedes the shared-foundation framing in ADR 0003 (items #4–#6)**. The findings and EV
   ranking in ADR 0003 stand; the "build one rollback store that all three consume" sequencing does not.

## Why

The trust boundary Deckard documents is the **uid**: code running as your user (including filesystem
write, `ptrace`, `/proc/<pid>/mem`) is trusted, and the wallet does not defend against live malware
running as you during an unlocked session, because the seed must enter RAM to sign (`THREAT-MODEL.md`,
`SECURITY.md`). Rollback requires writing your files, which is same-uid, which is already inside that
conceded zone. So the most this can ever defend is an attacker **weaker** than the one the model
already concedes (a careless backup/restore, a sync conflict, a sandboxed or limited process, offline
file theft).

For the **vault specifically**, the value is thin even against that weaker attacker:

- The seed is constant across re-seals, and balances live on-chain. Rolling `vault.bin` back gives the
  attacker no key and reverts nothing financial. Its entire worst case is reverting a **passphrase
  rotation or KDF-cost upgrade** to a previously-valid one.
- An attacker who can do that, during an unlocked session, can already read the live seed out of the
  daemon's RAM, which the threat model concedes. So this is a baroque, low-yield path to something
  the conceded attacker already has.

And the mechanism doesn't hold even where it was supposed to (adversarial findings, all verified):

- **Nothing legitimately advances the vault's counter.** Every `seal()` mints a fresh random
  `vault_id` (`keystore.rs`) and there is no re-seal-preserving-identity path; "upgrade-on-unlock
  re-seal" is a doc comment, not code. So the detector is vacuous until a counter that actually
  increments exists.
- **A plaintext / sidecar counter is replayable** (rolled back together with the vault, both validly
  authenticated). Only binding it into the AEAD associated data makes it non-forgeable, and that is a
  format-version break that retires the frozen on-disk known-answer fixtures.
- **The anchor is itself same-uid-reachable.** A sidecar file is deletable (deletion routes to a
  "new machine" bootstrap with no challenge); an OS-keychain item is rewritable. The only store a
  same-uid attacker can't roll backward is a **TPM 2.0 NV monotonic counter** — a new C-library
  dependency plus NV provisioning and headless capability probing, and macOS has **no app-facing
  equivalent**, so it would be a Linux-only security promise we couldn't keep cross-platform.

For a `0.0.1-alpha`, testnet-keys-only wallet, this is a large, breaking, partly-non-portable build to
defend a minor integrity property against a sub-boundary attacker. The cost is wildly out of line with
the value.

## Revisit conditions (the blockers)

Reopen this only when one clears:

- **B1 — the threat model rises.** The only attacker this stops is strictly weaker than the conceded
  same-uid attacker. Revisit if Deckard holds real/mainnet funds, becomes multi-user, or treats
  untrusted backup/sync as a first-class adversary.
- **B2 — a real counter exists.** Nothing advances the vault epoch today. This needs a
  re-seal-preserving-identity path *and* an AEAD-AAD-bound epoch (a vault format v2 that breaks the
  frozen compat fixtures). If an unrelated change forces a format bump anyway, anti-rollback can ride
  along.
- **B3 — a true anchor exists on all target platforms.** Today only a TPM NV counter is
  decrement-proof, and macOS lacks an app-facing equivalent. Don't ship rollback protection as a
  cross-platform guarantee until every supported OS has one.

## What we do instead (independent of `#71`)

Both close real holes in our *own* integrity story that a sub-same-uid attacker can hit today. Neither
imports an anchor; neither needs a format break.

- **`#108` — durable daily cap (highest value).** The cap is in-memory and zeroed on load, so a
  same-uid attacker crash-loops the daemon to reset it and drains in within-cap chunks
  (`policy_store.rs`, `daemon.rs`). Fix: **persist the spent counter, reserve-before-sign** (decrement
  durably *before* broadcast, reconcile by deterministic tx identity `chain + from + nonce`), survive
  restart, and make the day rollover **forward-only** so a backward clock can't reset it. This is a
  local-durability fix; it owes a same-uid attacker only tamper-*evidence*. Bolting it onto a
  cross-domain anchor would *create* the rollback surface, not close it.
- **`#72` — authenticate `policy.json`.** The agent-spending rulebook is plaintext and silently
  editable (`policy_store.rs`). Fix: a **MAC** keyed from vault material, **fail closed** on a
  bad/missing tag (extending the existing loud fallback), plus an inert monotonic **version field**.
  This closes forgery / silent edit. Anti-*replay* of an old valid policy is the same deferred bucket
  as `#71` (it only binds via the AAD break and only beats the weaker attacker), so we do not chase it
  now.

## Consequences

- **Positive:** we don't build a large, breaking, partly-non-portable mechanism for a minor property
  outside our threat boundary; the two issues with real teeth get smaller, faster, dependency-free
  fixes; the record stops a re-proposal of the same idea, and stops `#72`/`#108` being blocked on
  `#71`.
- **Cost:** vault rollback (and policy/cap anti-replay) remain undefended against a sub-same-uid
  attacker — accepted for alpha and recorded in `THREAT-MODEL.md`.
- **Deferred:** everything in the anchor family — the epoch, the keychain mirror, the TPM counter, the
  vault format v2, the re-seal-identity path — behind B1–B3.

## Status / next step

**Deferred.** `#71` stays open as a parked issue carrying B1–B3. `#72` and `#108` are decoupled and
proceed as independent local fixes (comments updated). No code ships from this spike.
