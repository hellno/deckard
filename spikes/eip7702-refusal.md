# Spike prompt: EIP-7702 detect-and-refuse (Deckard PRD-02 #46 · R1-b)

> Feed this whole file to a fresh coding agent. It is self-contained. Goal: settle
> **how an EIP-7702 "delegate my whole account" authorization could reach Deckard's
> message-signing surface, what exact bytes we refuse, and that the refusal predicate
> is backend-neutral** — so PRD-02 (#46) can ship `DELEGATION_REFUSED` and the shared
> 7702-auth chokepoint #33 will later extend. Standalone spike — do **not** touch the
> app crates (`crates/`). Output is a decision + test vectors, **not** merged code.

## Why this spike exists

#46 (PRD-02) must "refuse EIP-7702 delegation by default" and show "a distinct alarming
screen." #33 (session keys) is the *same 7702 primitive from the other side* — it will
later *allow* a scoped, self-originated grant. Per the maintainer cross-link on #46 they
must share **one** chokepoint, not two that drift. But before any code: a 7702
authorization is **neither** EIP-191 (`personal_sign`) **nor** EIP-712
(`eth_signTypedData_v4`) — it is a distinct tuple with its own `0x05` magic. So it is
**unknown whether/how one even arrives** through PRD-02's two signing methods, which means
the `delegation_refused` acceptance test currently has no defined trigger. This spike
defines that trigger and proves the refusal.

## Context you can trust (already verified this session — don't re-litigate)

- **The 7702 types are already in the tree — no new dependency.** A throwaway probe
  compiled clean (`cargo test -p deckard-core --test … --no-run`, Finished, EXIT=0):
  ```rust
  use alloy::eips::eip7702::Authorization;          // { chain_id: U256, address: Address, nonce: u64 }
  let h = Authorization { chain_id, address, nonce }.signature_hash();  // keccak256(0x05 ‖ rlp([chain_id,address,nonce]))
  ```
  `alloy-eip7702` is in `Cargo.lock` (≈line 402) via `alloy-eips`, and the `eips` feature
  is already enabled on the alloy meta-crate (`crates/deckard-core/Cargo.toml:38`). Use
  `alloy::eips::eip7702::{Authorization, SignedAuthorization}` directly. **Do not add a dep.**
- **The shape we refuse.** A 7702 authorization is the tuple `(chain_id, address, nonce)`
  signed over `keccak256(0x05 ‖ rlp([chain_id, address, nonce]))`. It rides in a **type-4
  (SET_CODE, `0x04`) transaction's `authorization_list`**. The authorization itself is
  signed *off-chain* — that off-chain signature is the thing a malicious dapp would try to
  extract from a wallet.
- **#33's frozen invariants (the chokepoint must carry these; this spike only exercises the
  refuse subset):** never sign `chain_id == 0` (spec-valid on *all* chains → unconditional
  refusal); delegate address MUST be on a **hardcoded, versioned audited-delegate allowlist**
  (user/dapp-supplied delegates rejected unconditionally); self-sponsored grants sign auth
  nonce **+1**; un-delegation to `address(0)` clears code **not storage**. The
  nonce+1 / storage rules are #33's *allow* path — **out of scope here.**
- **anvil scaffolding exists.** `crates/deckard-signerd/tests/common/mod.rs:148-221` has
  `start_anvil()` / `start_anvil_fork()`. 7702 needs a Prague-or-later EL — launch anvil
  with `--hardfork prague` (skip-gracefully when anvil is absent, like `anvil_e2e.rs:33`).
- **Working spike references:** `spikes/eip1193-railgun/` and `spikes/helios-walkaway/` —
  reuse their standalone-`[workspace]` `Cargo.toml` shape and the gitignore pattern.

## The question the spike must settle

> Through PRD-02's signing surface (`SignMessage::PersonalSign` / `SignMessage::TypedDataV4`),
> can a 7702 authorization reach the daemon — and if so, in what exact representation? Then:
> what is the minimal, **backend-neutral** predicate that refuses it (magic `0x05` recognized;
> `chain_id == 0` always; delegate ∉ allowlist always; refuse-by-default), and where does that
> predicate live so #46 and #33 share it?

## Tasks (do in order)

1. **Pin the alloy 7702 surface.** Confirm `Authorization`, `SignedAuthorization`,
   `signature_hash()`, RLP encoding, and the `0x05` magic constant against the
   `alloy-eips`/`alloy-eip7702` source at the locked version. Write down the exact types and
   the preimage. (Verified reachable above — pin the API.)
2. **Survey the trigger shape (the crux).** Determine how — if at all — a dapp delivers a
   7702 authorization for signing via standard provider methods. Confirm it is **not** EIP-191
   and **not** EIP-712. Document any experimental `wallet_*`/`eth_signAuthorization`-style
   methods and their request JSON. Conclude with **one** of:
   - (a) *No standard path* → PRD-02's refusal is a **smuggle guard**: detect a `0x05`-magic
     preimage (or a raw 32-byte hash) presented through `personal_sign`/`eth_sign` and refuse;
   - (b) *A method exists* → PRD-02 needs an explicit `SignMessage::Authorization7702 { … }`
     wire variant — specify its fields.
3. **Prove the mechanism on anvil.** Standalone bin: `anvil --hardfork prague`, sign an
   `Authorization`, send the type-4 tx, assert the EOA gains code (`eth_getCode` non-empty),
   then un-delegate to `address(0)` and assert code clears. (Seeds #33's S2/S7 later.)
4. **Build + prove the refusal predicate.** Write throwaway
   `refuse_7702_authorization(auth, allowlist) -> Decision` and run a vector table:
   `chain_id == 0` → refuse; delegate ∉ empty allowlist → refuse; a MetaMask-style delegate →
   still refuse; a Porto-style delegate → still refuse. Prove the verdict does **not** depend
   on which backend's delegate address is used (backend-neutrality).
5. **Sketch the chokepoint API + recommend the trigger.** Propose the public surface of
   `deckard-core/src/eip7702_auth.rs` (`is_authorization(...)`, `refuse(...)`, a *versioned,
   empty* `AUDITED_DELEGATES`), naming what #46 calls (refuse-all) vs what #33 later adds
   (allowlist contents + allow path). Recommend the `delegation_refused` trigger shape (2a/2b).

## Constraints

- Standalone crate at `spikes/eip7702-refusal/` with its own `[workspace]`; `.gitignore`
  `target/` + `Cargo.lock`. Do **not** edit anything under `crates/` or the app.
- **Refuse/read only — never sign a real 7702 authorization with a funded key.** The anvil
  round-trip uses anvil's throwaway dev keys; no mainnet, no real funds.
- No allowlist *contents*, no `nonce+1`/grant logic, no UI — those are #33 / PRD-02 proper.
- Never log or `Debug`-print a key or signature (repo rule; `Zeroizing`).

## Success criteria (what "done" means)

- A clear answer to the trigger question (2a vs 2b) with the exact request shape, **and** a
  YES/NO on "does a 7702 auth reach PRD-02's surface through standard methods?"
- A runnable anvil bin that sets and clears a delegation (mechanism understood end to end).
- A `vectors.json` of signed authorizations + expected refusal verdicts (mirrors
  `crates/deckard-signerd/tests/fixtures/cow/vectors.json`) — the seed for the
  `delegation_refused` acceptance test in #46.
- A short report + the `eip7702_auth.rs` API sketch with the #46-vs-#33 ownership split.

## Report format (return this)

```
<spike_report>
  <verdict>trigger: NO-STANDARD-PATH (smuggle guard) / METHOD-EXISTS (needs wire variant)</verdict>
  <trigger_shape><!-- exact request JSON / preimage we detect + refuse --></trigger_shape>
  <refusal_predicate_backend_neutral>YES / NO + evidence</refusal_predicate_backend_neutral>
  <chokepoint_api><!-- eip7702_auth.rs public surface; #46 refuse-all vs #33 allow-path --></chokepoint_api>
  <anvil_proof><!-- delegate set + cleared, with the cmds --></anvil_proof>
  <no_new_dep>confirmed (alloy eips) / NEW DEP NEEDED (why)</no_new_dep>
  <what_worked/>
  <what_failed/>
  <recommendation><!-- one paragraph for #46's delegation_refused test + the shared module --></recommendation>
  <artifacts><!-- spike crate path + how to run + vectors.json --></artifacts>
</spike_report>
```
