# ADR 0003 — Crate trust boundary: why the split exists, and what to harden first

- **Status:** Proposed (2026-06-20). Records the architecture defense + a re-prioritized hardening
  plan. Decisions only; executable work stays in GitHub issues (the hardening set `#71`–`#78`).
- **Deciders:** @hellno (maintainer)
- **Method:** source-grounded review (every load-bearing fact below is cited `file:line` and was
  grepped on branch `hellno/crate-architecture-overview`), `/cso` threat-model lens, and a two-pass
  cross-model adversarial review (codex GPT-5.x xhigh, run twice). Where codex was wrong it is
  corrected from source and the correction is recorded.
- **Context inputs:** `THREAT-MODEL.md`, `SECURITY.md`, `MANIFESTO.md`,
  [`ADR 0002`](0002-agent-wallet-and-session-keys.md), the hardening issues `#71`–`#78`.

## The question

"Does the `deckard-core` / `deckard-signerd` split make sense, and what should we change now,
fundamentally, to protect it as an open-source project — and what should we get audited first?"

The split is actually **two** decisions, and only one is load-bearing:

- **Decision A — `signerd` is a separate OS process** that solely holds the live, decrypted key at
  runtime. (The process boundary.)
- **Decision B — the keystore crypto lives in the fat shared `deckard-core` crate** that every
  interface links. (Where the code is compiled.)

The prior analysis made Decision B "the weak point" and recommended extracting a `deckard-keystore`
crate as the headline fix. **This ADR concludes that ranking is wrong.** Decision B is real hygiene,
but it is not a fund-loss path. The highest-expected-loss problems are in `signerd`'s launch
provenance, runtime hardening, and policy/cap integrity — and the cheapest practical attacks bypass
both the keystore envelope and the process split without ever touching crypto.

## Scope honesty (read this before the findings)

Every finding below sits **inside the same-uid local threat model** that `THREAT-MODEL.md` already
names as the boundary. Against a same-uid attacker (a malicious process, a supply-chained dependency,
or a prompt-injected agent driving the MCP surface) **no crate boundary fully saves you.** These
findings are about shrinking blast radius and making the boundary real and legible — not about a
remote exploit. Deckard is `0.0.1-alpha`, unaudited, testnet-keys-only (`SECURITY.md`); the point of
this ADR is to sequence the work that makes "same-uid is the boundary" actually true.

## The defense — why these crates exist (steelmanned, and it mostly holds)

This is the answer to "does the split make sense." Yes — the **runtime** boundary is well-built:

1. **One key-holder.** Every other crate is key-less and reaches the key only by proposing an
   `Intent` over a socket. `signerd` is the single process that ever constructs an `UnlockedVault`
   (`daemon.rs:31`, unlock at `daemon.rs:349-378`).
2. **One decision function, no drift.** `evaluate(intent, policy)` is shared in the contract crate
   (`policy.rs:64-111`); the mock and the real daemon call the same code, so verdicts can't diverge.
3. **You can't self-approve.** Approval (`Resolve`) is honored **only** on a private `socketpair` fd
   the app inherits at spawn (`DECKARD_RESOLVE_FD`, `supervise.rs:154-168`); on the public socket a
   `Resolve` returns `Deny "RESOLVE_NOT_AUTHORIZED"` (`daemon.rs:258-273`). The fd discipline is
   careful: the app end is `CLOEXEC` (`supervise.rs:163`) and the daemon re-arms `FD_CLOEXEC` on the
   inherited fd right after adopting it (`server.rs:110-111`).
4. **Clear-signing has no display/sign TOCTOU inside the app's own flow.** The recipient is
   snapshotted at review time and carried through hold-to-confirm (`commit_flow.rs:16-36`,
   `shell.rs:1664-1729`); epoch guards fence stale proposals.
5. **Mainnet guardrail.** On chain 1, every auto-`Allow` downgrades to `NeedsApproval`
   (`daemon.rs:567-569`, `mainnet_guardrail_active()` at `daemon.rs:1233-1234`).

So **Decision A is correct — keep it unconditionally.** One reframing, though: the split is an
**integrity and auditability** boundary (one auditable choke point that signs; a bug in the CoW HTTP
client is not a bug in the signer), **not a confidentiality** boundary against same-uid. Selling it
as "the live key is safe from local attackers" is marketing the split can't back (see F-C1, F-C5).

## Verified findings (severity / confidence — all VERIFIED against source)

| # | Sev | What | Evidence |
|---|-----|------|----------|
| **C1** | **CRITICAL** | Daemon spawn has **no `env_clear()`** and resolves the binary via env/PATH/sibling. A same-uid attacker injects `LD_PRELOAD`/`LD_AUDIT`/`DYLD_INSERT_LIBRARIES` into the key-holder, or substitutes the daemon binary to capture the forwarded passphrase. **Bypasses both the keystore envelope and the process split.** | `supervise.rs:270-280` (only 4 `.env()`, no `env_clear`), `supervise.rs:33-46` (`DECKARD_SIGNERD_BIN` > sibling-of-exe > bare `deckard-signerd` on PATH) |
| **C2** | **HIGH** | `policy.json` is plain JSON on disk, **same-uid-writable, unsigned, no integrity tag, no writer in our code**. Poison it (huge caps, `allow_to: []` = any recipient), force a respawn → daemon reloads the poisoned file. | `policy_store.rs` (read-only loader; no app writer found), missing-file = silent default, parse-error = loud default |
| **C3** | **HIGH** | The daily cap is not durable. `spent_today_wei` is **in-memory, zeroed on load** and incremented only post-broadcast; the supervisor auto-respawns on crash → crash-loop resets the cap and drains in within-cap chunks. | `policy_store.rs:5,63`, `daemon.rs:1068` (increment), `daemon.rs:1242` (reset), `supervise.rs:258-294` (respawn) |
| **C4** | **HIGH** | The approval-forcing guardrail is **chain-1 only**. Every L2 (Base/Arbitrum/Optimism/Polygon — real money) and testnets get no auto-allow downgrade. | `daemon.rs:1234`: `self.cfg.chain_id == 1 && !self.cfg.mainnet_override` |
| **C5** | **MEDIUM** | No `mlock`, `RLIMIT_CORE=0`, `PR_SET_DUMPABLE(0)`/`MADV_DONTDUMP` anywhere; macOS releases are unsigned/unhardened. So `ptrace`/`/proc/<pid>/mem`/core-dump/swap reach the live key. `THREAT-MODEL.md` already concedes the ptrace residual; the gap is it folds env/loader injection (C1) into "arbitrary same-uid code" when that vector is materially cheaper than ptrace. | grep: zero anti-inspection hardening; `SECURITY.md` defers Apple signing |
| **F1** | **MEDIUM** | `keystore.rs` is not feature-gated; **every** core-linking binary compiles seal/unlock/HD-derivation (mcp, wallet-client, transitively browser-bridge). You can't read "which binary can decrypt" off `cargo tree`. Hygiene/legibility, not a fund-loss path. | `lib.rs:49` (`pub mod keystore;`, no gate) |
| **F2** | **MEDIUM→HIGH** | The seed is sealed **inside the GPUI app**, which links core with **default features ON** (alloy + reqwest + helios/revm/bls + railgun ZK + cow_client). The plaintext mnemonic — the recovery root, worth more than the live key — transits the fattest-dependency process at its most sensitive moment. | `shell.rs:721/855/857/951` (`Vault::create`/`import_*`), app `Cargo.toml` has no `default-features = false` |

**Fund-loss ranking (highest expected loss first):** C1 (cheapest full compromise) ≈ C2/C3/C4
(hands-free drain, no key theft) > C5 (live-key capture) > F1/F2 (hygiene/blast-radius). **Keystore
placement (F1) is not on the fund-loss list at all.**

### Cross-model corrections recorded (the adversarial method working)

- codex pass 1 claimed the **default daily cap is 1 ETH**. **Wrong** — `DEFAULT_DAILY_CAP_WEI = 0.2
  ETH`, `DEFAULT_PER_TX_CAP_WEI = 0.05 ETH` (`policy_store.rs:17,19`). Not propagated.
- codex pass 1 ranked the **clear-signing UI last** to audit; pass 2 self-corrected: because the
  mainnet guardrail forces *every* auto-allow into human approval, the approval surface is a primary
  control on mainnet, not a garnish. Adopted (see audit priority).
- codex's proposed fix "have signerd return the mnemonic to the GUI to display" is **oversold**: if
  the GUI displays the words, the plaintext is back in the fat process. The honest F2 fix is a minimal
  forked helper, or accepting the window and hardening the process it lives in.
- The off-mainnet **per-intent-kind** auto-broadcast behavior (C4 → drain path) has nuance in
  `daemon.rs:463-507` (Send vs Shield handling) that we did not fully trace. C4 and C3 are
  individually verified facts; their exact combined drain path off-mainnet is flagged **for the
  audit to confirm**, not asserted as a finished exploit.

## Decision — what to change, in priority order

**Keep Decision A unconditionally.** Then, in EV order (not the prior "extract keystore first"):

### Tier 0 — launch provenance (closes C1)
1. `env_clear()` the daemon child, set only the four needed vars; **test** that loader vars
   (`LD_PRELOAD`/`LD_AUDIT`/`DYLD_INSERT_LIBRARIES`) and inherited `PATH` cannot reach the child.
   Prefer the private fd channel over env for anything sensitive. (Closes the loader-injection half of
   C1. Nearly free.)
2. In release builds, resolve the daemon to **one canonical, bundled path** with ownership/permission
   + symlink + signature/hash verification *before exec*; the `DECKARD_SIGNERD_BIN`/PATH/sibling
   override must be **impossible to compile into** a release artifact (dev/test feature only, not
   merely runtime-disabled), so demo/qa keep working. (Closes the binary-substitution half of C1.)

### Tier 0 — runtime memory hardening (closes C5) — separate workstream
3. Anti-inspection: `mlock` **all** live secret buffers + temp copies, `RLIMIT_CORE=0`,
   `PR_SET_DUMPABLE(0)`/`MADV_DONTDUMP` (Linux); Hardened Runtime + no `get-task-allow` + signed
   release (macOS, depends on the signing workstream). **Fail loudly** if a hardening call fails —
   don't silently continue. Platform-specific, different validation + release deps than C1, so it
   ships as its own issue. (`unsafe`/FFI lives here — another reason it belongs in `signerd`.)

### Tier 0 — FOUNDATION: one rollback-resistant security-state store (keystone for C2 + C3)
4. **Build the unified store first.** Both policy (C2) and cap accounting (C3) are "authenticated,
   monotonic, transactionally-updated security state on disk." Authenticity alone does **not** stop
   replay of an older valid (more-permissive) state, so the store needs a **monotonic version/epoch
   anchored outside the file** (this is `#71`), atomic write + `fsync` + dir-sync, fail-closed
   partial-write recovery, single-writer locking, and domain-separated key derivation. C2 and C3
   depend on this; sequencing them without it is the **single biggest gap** the codex pre-flight
   found.

### Tier 0 — policy + cap integrity (on the foundation; closes C2, C3, C4)
5. **C2 — authenticated policy.** A **MAC** (vault-derived *symmetric* key → keyed MAC/AEAD tag, **not**
   a "signature") over `policy.json` **plus the monotonic version** from #4; `signerd` **fails closed**
   on a bad/missing tag, stale version, or non-`0600`/symlinked/non-owned file. Policy stays **on disk,
   not in the vault** (respects `#72`'s locked decision). Define who authorizes a policy change + the
   first-run bootstrap. *(mechanism confirmed 2026-06-20; MAC/rollback corrected per codex)*
6. **C3 — reserve cap before signing.** Durably **reserve** the spend in the rollback-resistant store
   *before* releasing the signature/broadcast (post-broadcast increment leaves a crash window);
   reconcile unknown-outcome / dropped / replaced / reorg'd broadcasts; bind accounting to
   chain + account + policy-version + UTC window. Persisting `spent_today_wei` alone is **not enough**.
   *(corrected per codex)*
7. **C4 — per-chain opt-in, fail closed.** Real-money chains require approval by default; **unknown /
   custom chains default to approval-required**, and the classification must not trust a mutable
   registry entry or an RPC-reported chain id alone. Testnets/forks stay hands-free for the demo.
   Hands-free enablement is itself authenticated + rollback-protected (depends on #4). Source the trust
   tier from the chain capability registry (`#97`). *(confirmed 2026-06-20)*

### Tier 1 — dependency-graph legibility (closes F1) — three *separate* issues
8. **F1a (cheap):** feature-gate `keystore` in core (`keystore = []` + `pub mod keystore` behind it)
   so mcp/wallet-client/browser-bridge stop compiling decrypt code. Acceptance must inspect the actual
   release-binary feature/dep closure (Cargo feature unification can silently re-add it).
9. **F1b (optional follow-up, its own issue with a real done-criterion):** extract a minimal
   `deckard-keystore` crate (`#![forbid(unsafe_code)]`, deps = KDF + AEAD + BIP-39/HD only). Not a
   loose "optional" bullet on F1a.
10. **F2 (separate, honestly scoped):** the threat is *plaintext seed in the fattest process*. Moving
    only generation to a forked helper is **surface reduction, not removal** — if the GUI displays the
    words, the seed still enters the GUI. Either scope the issue as "reduce generation-time surface"
    *or* design onboarding so the seed never enters the GUI (a trusted display/confirm path). State
    which; don't claim F2 is "fixed" by the helper alone.

### Not doing
- **Not** moving onboarding wholesale into signerd. The human must see the recovery words; a naive
  GUI-display path puts plaintext back in the app regardless. F2 (item 10) is where that tension gets
  resolved honestly, not waved away.

## OSS protection — keep the boundary honest over time

The boundary rots via a future PR re-linking decrypt code or a dep creeping into the trust path.
Necessary (the cargo-tree check is necessary-but-gameable — it proves code-absence, not capability):

- **Per-binary feature/dep-closure check** (custom `cargo metadata` script, NOT `cargo-deny [bans]`):
  while keystore is a feature-gated *module* there is no package to ban, so the guard must compute each
  release binary's resolved feature+dep closure and fail if keystore appears in
  `mcp`/`wallet-client`/`browser-bridge`. `cargo-deny [bans]` becomes available only **after** F1b
  extraction (then it's a real package). Must survive re-exports and feature unification.
- **`cargo-vet`** gating new/updated deps into the trust path; re-run on every `just bump-gpui`
  (that's the moment a transitive crate creeps in).
- **`CODEOWNERS` + branch protection** — CODEOWNERS alone is not a security boundary; it only enforces
  if branch protection *requires* code-owner review (no bypass) on the protected branch. Cover the
  keystore crate, `signerd` daemon/policy/socket/supervise, `contract/src/policy.rs`, `deny.toml`. The
  branch-protection settings are part of the acceptance criteria, not an afterthought.
- **Reproducible + signed releases** (also the macOS-hardening unblock from C5).
- **The real boundary is runtime, not build-time:** `signerd` self-checks — refuse a non-`0600`
  vault, refuse an unsigned policy, pin the vault path. These enforce capability; CI enforces only
  code presence.

This sharpens, not replaces, the existing hardening issues `#71`–`#78` (policy-integrity, cargo-vet,
guardrail/multi-chain, HW-backed keys map onto C2/C4/C5 and the CI work above).

## Audit priority — what to pay for first

Highest expected loss first. The keystore is standard primitives wired together (low novelty); the
bespoke, hands-free, money-moving logic is where novelty × money × agent-exposure peaks.

1. **Policy gate + Intent admission** (`daemon.rs` decision path): the guardrail's chain-1 scope (C4),
   the restart-resettable cap (C3), on-disk policy integrity (C2), cap re-check semantics, swap
   admission. This is where money moves hands-free for agents. **Audit first.**
2. **Launch provenance + IPC capability model** (`supervise.rs`, `server.rs`, env handling): the
   `env_clear`/binary-resolution Critical (C1) and the resolver-fd discipline. Cheapest path to
   key/approval compromise.
3. **CoW/Railgun calldata builders + the clear-signing UI, together**: does the daemon build exactly
   what the human approved? A wrong approve-spender or shield target routes funds; the UI is the
   load-bearing control on mainnet (guardrail forces approval), so audit "what the human sees vs. what
   gets signed" as one unit.
4. **Keystore envelope** (Argon2id + XChaCha20-Poly1305 + SLIP-0010/BIP-39): standard, KAT-gated,
   bounded reader for untrusted bytes. Audit the construction, but it's the **best-built** part — last,
   not first.

## Consequences

- **Positive:** the process split keeps its real value (integrity/auditability) and stops being
  oversold; the cheapest real attacks (C1–C4) get closed first; the dependency graph becomes a true
  statement about the key-less binaries; the audit budget is spent where the money actually moves.
- **Cost:** Tier-0 adds `unsafe`/FFI to signerd (mlock/dumpable) and a signing/release pipeline; the
  rollback-resistant store (keystone) is the biggest single piece — a monotonic, atomic, fail-closed
  security-state store that policy (MAC + version) and cap (reserve-before-sign) both build on. All in
  `signerd`, but the keystone is real work, not a one-liner.
- **Deferred:** HW-backed DEK (Secure Enclave / TPM) and idle-lock are real long-term answers to cold
  theft and the unlocked-session window, tracked under the hardening set, not blocking Tier 0.

## Finding → issue map (revised 2026-06-20 after codex pre-flight)

The first draft bundled C1+C5 and F1+F2 and proposed `cargo-deny` bans + "persist the counter". The
codex pre-flight review split those, corrected the mechanisms, and surfaced the rollback keystone. Net:
**8 work items, not 5**, and a disclosure split (public repo).

| Item | Existing issue | Action | Disclosure |
|---|---|---|---|
| Rollback-resistant security-state store (keystone) | **#71** | refine: make it the explicit foundation for C2 + C3 (monotonic epoch, atomic+fsync, fail-closed, single-writer) | public (design) |
| C1 launch provenance | — | **new** (env_clear + tests; canonical/verified binary path; override compiled-out of release) | public, full detail |
| C5 runtime memory hardening | — | **new, separate** (mlock-all/coredump/dumpable; fail-loud; macOS needs signing) | public |
| C2 authenticated policy | **#72** | refine: **MAC** (not "signature") + monotonic version on the keystone; fail-closed; not-in-vault | public (design); keep poison detail thin |
| C3 reserve-before-sign cap | — | **new** (durable reservation pre-broadcast + reconciliation, on the keystone) | public, full detail |
| C4 per-chain guardrail | **#76** (+ **#97**) | refine: per-chain opt-in, **unknown chains fail-closed**, classification not RPC-trusted | public |
| Architecture-fitness CI | **#75** (cargo-vet) | **new** custom feature-closure check (not cargo-deny bans while module-gated) | public |
| Governance: CODEOWNERS + branch protection | — | **new, separate** from CI | public |
| F1a keystore feature-gate | — | **new** (closure-aware acceptance) | public |
| F1b keystore crate extraction | — | **new, optional follow-up** (own done-criterion) | public |
| F2 seed-seal surface | — | **new** (scope honestly: surface-reduction *or* seed-never-in-GUI) | public |

## Status / next step

**Proposed.** Reprioritization + the C1/C2/C4 mechanisms confirmed with the maintainer (2026-06-20);
issue set then hardened by a codex pre-flight (rollback keystone, C3 reserve-before-sign, MAC-not-
signature, CI feature-closure-not-bans). **Disclosure decided 2026-06-20: public, full detail** — this
is a testnet-only alpha and same-uid is already the documented boundary in `THREAT-MODEL.md`, so no
private-advisory split. Next: create the public issues + refinement comments per the map, all linking
this ADR, then promote to Accepted once the foundation + Tier-0 work lands.
