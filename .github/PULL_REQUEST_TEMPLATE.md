<!--
  Deckard is 0.0.1-alpha, security-sensitive software (it holds private keys, BIP-39 seeds,
  and an encrypted keystore). Please complete every section. PRs that leave the Definition of
  Done checklist unverified will not be merged.
-->

## Summary

<!-- What does this PR change, and why? Keep it focused. -->

## Linked issue

<!-- e.g. "Closes #123". If there is no issue, briefly explain the motivation here. -->

Closes #

## Definition of Done

All of the following must hold before this PR can merge. **Paste the command output as evidence —
do not check a box you have not verified.**

- [ ] `cargo fmt --all --check` is clean
- [ ] `just check` is green — clippy `-D warnings` on **both** the default config **and** `--features tray`
- [ ] `cargo test --workspace` is green
- [ ] No new or changed dependencies in `Cargo.toml` / `Cargo.lock` (unless explicitly approved in this PR;
      the git GPUI stack is bumped only via `just bump-gpui`, never hand-edited)
- [ ] Any visual/UI change follows `DESIGN.md` (amber = human, cyan = agent; sidebar/contextual-views IA;
      clear-signing / seed-reveal trust affordances)
- [ ] Secrets (seed / key / passphrase) stay in `Zeroizing` and are never logged or `Debug`-printed

<details>
<summary>Evidence (paste command output here)</summary>

```text
$ cargo fmt --all --check
# (output)

$ just check
# (output)

$ cargo test --workspace
# (output)
```

</details>

## Notes for reviewers

<!--
  Anything that helps review: security-relevant trade-offs, areas that need a careful look,
  follow-ups deliberately left out of scope, or test caveats (e.g. #[ignore] network tests).
-->
