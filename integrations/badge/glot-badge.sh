#!/bin/sh
# shields.io endpoint JSON for "translated by glot · N languages · X%".
#   glot-badge.sh                 print the JSON (run where glot.toml lives)
#   glot-badge.sh --push [BRANCH] commit it as glot-badge.json to BRANCH (default glot-badge,
#                                 created as an orphan branch) on origin
# Badge: https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/OWNER/REPO/glot-badge/glot-badge.json
set -eu

json=$(glot status --json | jq -c '
  [.locales[]] as $l
  | ($l | map(.translated) | add // 0) as $done
  | ($l | map(.total) | add // 0) as $all
  | (if $all == 0 then 100 else ($done * 100 / $all | floor) end) as $pct
  | {schemaVersion: 1, label: "translated by glot",
     message: "\($l | length + 1) languages · \($pct)%",
     color: (if $pct >= 95 then "brightgreen" elif $pct >= 70 then "yellow" else "red" end)}')

[ "${1:-}" = --push ] || { echo "$json"; exit 0; }

branch=${2:-glot-badge}
dir=$(mktemp -d)
trap 'git worktree remove --force "$dir" >/dev/null 2>&1 || true; rm -rf "$dir"' EXIT
if git fetch -q origin "$branch" 2>/dev/null; then
  git worktree add -q --detach "$dir" FETCH_HEAD
else
  git worktree add -q --detach "$dir"
  git -C "$dir" checkout -q --orphan tmp-glot-badge
  git -C "$dir" rm -rqf .
fi
echo "$json" > "$dir/glot-badge.json"
git -C "$dir" add glot-badge.json
if git -C "$dir" diff --cached --quiet; then echo "badge unchanged: $json"; exit 0; fi
git -C "$dir" -c user.name="glot[bot]" -c user.email="glot-bot@users.noreply.github.com" commit -qm "glot badge: $(echo "$json" | jq -r .message)"
git -C "$dir" push -q origin "HEAD:refs/heads/$branch"
echo "pushed $json to $branch"
