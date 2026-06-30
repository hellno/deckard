#!/usr/bin/env bash
# Enforce — and audit — branch protection on `main` so the real Definition-of-Done
# CI jobs block merge, not just the two cargo-deny checks.
#
# WHY (#123). Before this, branch protection on `main` required ONLY
# `cargo-deny-advisories` and `cargo-deny-supply-chain`. The actual DoD jobs —
# `quick` (fmt), `linux` (build + clippy -D warnings + cargo test --workspace +
# signerd tests), and `macos` (build + tray clippy) — existed in CI but were NOT
# required, so a PR could merge with the build red, tests failing, or clippy
# warnings. This script makes the full set required and turns on admin enforcement
# so the gate is real: an admin cannot click-merge through a red required check.
#
# This is the single source of truth for that policy. The contexts live in one
# bash array below; edit there, re-apply, done.
#
#   scripts/branch-protection.sh            # APPLY the desired state, then self-verify
#   scripts/branch-protection.sh --verify   # READ-ONLY audit; exits non-zero on drift
#
# Surgical by design: it PATCHes only required_status_checks and POSTs only
# enforce_admins. It never does a full-branch PUT, so required_signatures and the
# (deliberately absent) required_pull_request_reviews are left untouched — this is
# a solo-admin auto-merge repo; adding required reviews would break that workflow.
#
# Decision — strict=false is deliberate. The issue lists "require branches to be up
# to date before merging" as OPTIONAL. On a fast solo repo `strict=true` forces a
# rebase before every merge and slows the auto-merge flow, so we leave it off. To
# turn it on, flip STRICT below (a commented line shows where).
#
# ROLLBACK (back to the pre-#123 state):
#   gh api -X DELETE repos/hellno/deckard/branches/main/protection/enforce_admins
#   gh api -X PATCH  repos/hellno/deckard/branches/main/protection/required_status_checks \
#     -f strict=false -f 'contexts[]=cargo-deny-advisories' -f 'contexts[]=cargo-deny-supply-chain'
#
# Deps: gh (authenticated, with admin on the repo) and jq. Both fail loudly if missing.
set -euo pipefail

# --- Single source of truth -------------------------------------------------
REPO="hellno/deckard"
BRANCH="main"

# Desired required status checks. The two cargo-deny jobs were already required;
# the three DoD jobs (quick/linux/macos) are what #123 adds. Union, sorted is the
# verify target — order here does not matter, the verify normalizes it.
DESIRED_CONTEXTS=(
  "cargo-deny-advisories"
  "cargo-deny-supply-chain"
  "quick"
  "linux"
  "macos"
)

# strict = "require branches up to date before merging". Deliberately false (see
# the header). To require up-to-date branches, set this to "true":
STRICT="false"
# STRICT="true"   # <-- uncomment to require branches be up to date before merge

# --- Preconditions ----------------------------------------------------------
for bin in gh jq; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "ERROR: '$bin' not found on PATH. Install it (gh: https://cli.github.com, jq: https://jqlang.github.io) and re-run." >&2
    exit 2
  fi
done

PROTECTION="repos/$REPO/branches/$BRANCH/protection"

# Sorted, newline-joined copy of the desired contexts — the comparison target.
desired_sorted="$(printf '%s\n' "${DESIRED_CONTEXTS[@]}" | sort -u)"

# --- Verify (read-only) -----------------------------------------------------
# Asserts (a) required_status_checks.contexts == desired set and (b)
# enforce_admins.enabled == true. Prints a PASS/FAIL line per assertion; exits 1
# if either fails. Used standalone (--verify) and as the apply self-check.
verify() {
  local protection got_sorted enforce_admins fails=0

  protection="$(gh api "$PROTECTION")"

  # GitHub returns contexts under required_status_checks.contexts (and mirrors them
  # in .checks[].context); read the canonical .contexts list.
  got_sorted="$(printf '%s' "$protection" \
    | jq -r '.required_status_checks.contexts[]?' | sort -u)"

  if [ "$got_sorted" == "$desired_sorted" ]; then
    echo "PASS  required_status_checks.contexts == desired set ($(printf '%s' "$desired_sorted" | paste -sd, -))"
  else
    echo "FAIL  required_status_checks.contexts drift:" >&2
    echo "        want: $(printf '%s' "$desired_sorted" | paste -sd, -)" >&2
    echo "        got:  $(printf '%s' "$got_sorted" | paste -sd, -)" >&2
    fails=1
  fi

  enforce_admins="$(printf '%s' "$protection" | jq -r '.enforce_admins.enabled')"
  if [ "$enforce_admins" == "true" ]; then
    echo "PASS  enforce_admins.enabled == true"
  else
    echo "FAIL  enforce_admins.enabled == $enforce_admins (want true)" >&2
    fails=1
  fi

  if [ "$fails" -ne 0 ]; then
    echo "branch-protection: VERIFY FAILED for $REPO@$BRANCH — run 'scripts/branch-protection.sh' to converge." >&2
    return 1
  fi
  echo "branch-protection: VERIFY OK — $REPO@$BRANCH gated by the full DoD checks with admin enforcement." >&2
}

# --- Apply (idempotent) -----------------------------------------------------
# PATCH the required_status_checks endpoint with the full desired contexts list,
# then POST to enable enforce_admins. Both are convergent: re-running sets the same
# state without error. Neither endpoint touches required_signatures or
# required_pull_request_reviews.
apply() {
  echo "branch-protection: applying desired state to $REPO@$BRANCH ..." >&2

  # Build the required_status_checks payload as JSON so the contexts array is exact
  # (avoids -f 'contexts[]=' append ambiguity), then pipe it in.
  jq -n --arg strict "$STRICT" --args '
    { strict: ($strict == "true"), contexts: $ARGS.positional }
  ' "${DESIRED_CONTEXTS[@]}" \
    | gh api -X PATCH "$PROTECTION/required_status_checks" --input - >/dev/null

  # Enabling admin enforcement is a bodyless POST; DELETE on the same endpoint
  # disables it (see ROLLBACK). POST is idempotent — re-enabling stays enabled.
  gh api -X POST "$PROTECTION/enforce_admins" >/dev/null

  echo "branch-protection: applied; verifying ..." >&2
  verify
}

# --- Dispatch ---------------------------------------------------------------
case "${1:-}" in
  ""|--apply)
    apply
    ;;
  --verify|--check)
    verify
    ;;
  -h|--help)
    sed -n '2,33p' "$0"
    ;;
  *)
    echo "usage: scripts/branch-protection.sh [--apply | --verify]" >&2
    exit 2
    ;;
esac
