#!/usr/bin/env bash
# pr-image.sh — publish local image(s) to the repo's `assets` branch and print
# paste-ready GitHub markdown that renders inline in an issue / PR / comment.
#
# WHY THIS EXISTS
#   `gh` cannot upload an image into GitHub markdown — that path is the web UI's
#   private upload endpoint. But GitHub renders `![](url)` for ANY URL that returns
#   image bytes, and a PUBLIC repo serves its own files at raw.githubusercontent.com.
#   So we park screenshots on a dedicated orphan `assets` branch (never merged, never
#   in a PR diff, never in the working tree) and reference their raw URLs. This is the
#   standard CI trick and it is fully scriptable.
#
# HOW IT WORKS (pure plumbing — does NOT touch your working tree, index, or HEAD)
#   hash-object -w  →  merge into the existing assets tree  →  commit-tree  →  push.
#
# USAGE
#   scripts/pr-image.sh <image> [<image> ...]
#   scripts/pr-image.sh --prefix 198 before.png after.png   # names → 198-before.png …
#
#   Then paste the printed markdown into `gh pr edit --body-file` / `gh pr comment`.
#
# REQUIREMENTS
#   - a PUBLIC repo (raw URLs need no auth). A private repo's raw URLs won't render
#     for viewers — this script refuses to run against one.
#   - an `origin` remote on GitHub.
set -euo pipefail

BRANCH="assets"
PREFIX=""
if [[ "${1:-}" == "--prefix" ]]; then
    PREFIX="${2:?--prefix needs a value}-"
    shift 2
fi
[[ $# -ge 1 ]] || { echo "usage: $0 [--prefix <p>] <image> [<image> ...]" >&2; exit 2; }

# Refuse a private repo: viewers can't render its raw URLs without a token.
vis=$(gh repo view --json visibility --jq .visibility 2>/dev/null || echo UNKNOWN)
if [[ "$vis" != "PUBLIC" ]]; then
    echo "error: repo visibility is '$vis' — raw URLs only render for a PUBLIC repo." >&2
    echo "  For a private repo, drag-drop the image into the PR web UI instead." >&2
    exit 1
fi

nwo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)

# Start the new tree from the existing assets branch (so prior images survive), or
# empty if the branch doesn't exist yet.
git fetch -q origin "$BRANCH" 2>/dev/null || true
declare -a tree_lines=()
parent_args=()
if git rev-parse -q --verify FETCH_HEAD >/dev/null 2>&1 && \
   git ls-tree "origin/$BRANCH" >/dev/null 2>&1; then
    while IFS= read -r line; do tree_lines+=("$line"); done < <(git ls-tree "origin/$BRANCH")
    parent_args=(-p "origin/$BRANCH")
fi

# Add each image as a blob, replacing any existing entry with the same name.
declare -a names=() urls=()
for img in "$@"; do
    [[ -f "$img" ]] || { echo "error: no such file: $img" >&2; exit 1; }
    name="${PREFIX}$(basename "$img")"
    blob=$(git hash-object -w "$img")
    # drop any prior entry with this name, then append the new one
    filtered=()
    for l in "${tree_lines[@]:-}"; do
        [[ -n "$l" && "$l" == *$'\t'"$name" ]] && continue
        [[ -n "$l" ]] && filtered+=("$l")
    done
    tree_lines=("${filtered[@]:-}")
    tree_lines+=("$(printf '100644 blob %s\t%s' "$blob" "$name")")
    names+=("$name")
    urls+=("https://raw.githubusercontent.com/$nwo/$BRANCH/$name")
done

tree=$(printf '%s\n' "${tree_lines[@]}" | git mktree)
commit=$(git commit-tree "$tree" "${parent_args[@]}" -m "assets: $*")
git push -q origin "$commit:refs/heads/$BRANCH"

echo "published to the '$BRANCH' branch. Paste-ready markdown:" >&2
echo >&2
for i in "${!urls[@]}"; do
    printf '![%s](%s)\n' "${names[$i]%.*}" "${urls[$i]}"
done
