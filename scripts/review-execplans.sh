#!/usr/bin/env sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
plans_dir="$repo_root/execplans"
stale_days="${1:-30}"
now="$(date +%s)"
stale_seconds=$((stale_days * 24 * 60 * 60))

if [ ! -d "$plans_dir" ]; then
  echo "execplans/ does not exist"
  exit 1
fi

echo "Reviewing execution plans in $plans_dir"
echo "Stale threshold: ${stale_days} days"
echo

plans="$(find "$plans_dir" -maxdepth 1 -type f -name '*.md' \
  ! -name 'README.md' ! -name 'TEMPLATE.md' | sort)"

if [ -z "$plans" ]; then
  echo "No execution plans found."
  exit 0
fi

printf '%s\n' "$plans" | while IFS= read -r plan; do
  rel="execplans/$(basename "$plan")"
  modified="$(stat -c %Y "$plan")"
  age_days=$(((now - modified) / 86400))

  status="ok"
  if ! grep -q '^## 13\. Status Notes' "$plan"; then
    status="missing-status-notes"
  elif [ $((now - modified)) -gt "$stale_seconds" ]; then
    status="stale"
  fi

  printf '%-28s %4sd  %s\n' "$status" "$age_days" "$rel"
done
