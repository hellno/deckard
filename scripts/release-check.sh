#!/usr/bin/env bash
# Validate a release tag against the workspace + CHANGELOG, then print the tag's
# CHANGELOG section to stdout. This is the single source of truth shared by
# `just release-check` and .github/workflows/release.yml, so the local pre-flight
# and CI run the exact same checks. Diagnostics go to stderr; the release body
# goes to stdout.
#
#   scripts/release-check.sh v0.0.2-alpha > RELEASE_BODY.md
#
# Exits non-zero (with an actionable message) when the tag is malformed, the crate
# versions don't all match the tag, or the CHANGELOG section is missing/empty.
# Deps: cargo, jq, awk — all present on GitHub ubuntu runners and locally.
set -euo pipefail

TAG="${1:-}"
if [ -z "$TAG" ]; then
  echo "usage: scripts/release-check.sh <tag>   (e.g. v0.0.2-alpha)" >&2
  exit 2
fi
VER="${TAG#v}"

# 1. Tag shape: vMAJOR.MINOR.PATCH, optional -alpha|-beta|-rc[.-]N suffix.
if ! printf '%s' "$TAG" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)([.-]?[0-9]+)?)?$'; then
  echo "ERROR: tag '$TAG' is not a 'vMAJOR.MINOR.PATCH[-alpha|-beta|-rc]' tag." >&2
  echo "  Fix: tag like 'v0.0.2-alpha'. See docs/RELEASING.md." >&2
  exit 1
fi

# 2. Every workspace crate version must equal the tag — no split-version release.
versions="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].version' | sort -u)"
if [ "$(printf '%s\n' "$versions" | grep -c .)" -ne 1 ] || [ "$versions" != "$VER" ]; then
  echo "ERROR: not every crate is at version '$VER':" >&2
  cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | "  \(.name) = \(.version)"' >&2
  echo "  Fix: set every crate manifest to $VER (docs/RELEASING.md §2), or tag the version the crates carry." >&2
  exit 1
fi

# 3. Extract the '## [VER]' CHANGELOG section. Anchor on the [version] bracket only
#    (the date delimiter differs between docs); stop at the next '## ' heading.
body="$(awk -v ver="$VER" '
  index($0, "## [" ver "]") == 1 { grab = 1; next }
  grab && (/^## / || /^\[[^]]+\]:[ \t]/) { exit }   # next version heading, or the link-ref footer
  grab { print }
' CHANGELOG.md)"

# Trim leading/trailing blank lines (portable awk — no GNU-only sed).
body="$(printf '%s\n' "$body" \
  | awk 'NF { p = 1 } p { print }' \
  | awk '{ a[NR] = $0 } END { last = NR; while (last > 0 && a[last] ~ /^[[:space:]]*$/) last--; for (i = 1; i <= last; i++) print a[i] }')"

if [ -z "$(printf '%s' "$body" | tr -d '[:space:]')" ]; then
  echo "ERROR: no '## [$VER]' section found in CHANGELOG.md (or it is empty)." >&2
  echo "  Fix: promote the [Unreleased] block to '## [$VER] - $(date +%F)' BEFORE tagging (docs/RELEASING.md §3)." >&2
  exit 1
fi

echo "release-check: $TAG OK — all crates at $VER; CHANGELOG section found." >&2
printf '%s\n' "$body"
