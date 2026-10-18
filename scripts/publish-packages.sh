#!/usr/bin/env bash
# Publish the glot npm packages, in dependency order, under one scope.
#
#   NPM_CONFIG_USERCONFIG=~/.npmrc-glot scripts/publish-packages.sh              # scope @glot-i18n
#   SCOPE=@glot NPM_CONFIG_USERCONFIG=~/.npmrc-glot scripts/publish-packages.sh  # if you own @glot
#   DRY_RUN=1 scripts/publish-packages.sh                                        # pack + `npm publish --dry-run`
#
# The repo names the packages `@glot/*`; each one is packed (prepack builds dist/), then its name and
# README are rewritten to $SCOPE in a staging copy, which is what gets published. `@glot` on npm belongs
# to someone else today: the glot binary looks for `@glot-i18n/visual-qa` / `@glot-i18n/agent-qa` unless
# built with `GLOT_NPM_SCOPE=@glot`. This script holds no token: the npmrc named by NPM_CONFIG_USERCONFIG does.
set -euo pipefail

SCOPE="${SCOPE:-@glot-i18n}"
DRY_RUN="${DRY_RUN:-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# runtime first (edge bundles it), the CLIs last (agent-qa bundles visual-qa's sources).
PACKAGES=(runtime overlay feedback insights db edge visual-qa agent-qa)

[[ "$SCOPE" == @* ]] || { echo "SCOPE must start with @ (got $SCOPE)" >&2; exit 2; }
if [[ -z "$DRY_RUN" ]]; then
  [[ -n "${NPM_CONFIG_USERCONFIG:-}" && -f "$NPM_CONFIG_USERCONFIG" ]] ||
    { echo "set NPM_CONFIG_USERCONFIG to an npmrc holding the publish token" >&2; exit 2; }
  echo "publishing as $(npm whoami) under $SCOPE"
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

for p in "${PACKAGES[@]}"; do
  dir="$ROOT/packages/$p"
  echo "── $SCOPE/$p"
  (cd "$dir" && pnpm install --frozen-lockfile --silent 2>/dev/null || pnpm install --silent)
  tgz="$(cd "$dir" && npm pack --silent --pack-destination "$STAGE" | tail -1)"
  mkdir -p "$STAGE/$p"
  tar -xzf "$STAGE/$tgz" -C "$STAGE/$p"
  pkg="$STAGE/$p/package"
  if [[ "$SCOPE" != "@glot" ]]; then
    # name + README only: no published file imports another @glot package (edge and agent-qa bundle theirs).
    node -e '
      const fs = require("fs"), [dir, scope] = process.argv.slice(1);
      const f = dir + "/package.json", j = JSON.parse(fs.readFileSync(f, "utf8"));
      j.name = j.name.replace(/^@glot\//, scope + "/");
      fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
      const r = dir + "/README.md";
      if (fs.existsSync(r)) fs.writeFileSync(r, fs.readFileSync(r, "utf8").replaceAll("@glot/", scope + "/"));
    ' "$pkg" "$SCOPE"
  fi
  if grep -rlq "@glot/" "$pkg/dist" 2>/dev/null && [[ "$SCOPE" != "@glot" ]]; then
    echo "$p: dist/ still references @glot/*" >&2; exit 1
  fi
  name="$(node -p 'require(process.argv[1]).name + "@" + require(process.argv[1]).version' "$pkg/package.json")"
  if [[ -n "$DRY_RUN" ]]; then
    (cd "$pkg" && npm publish --dry-run --ignore-scripts --access public 2>&1 | grep -E "name:|version:|total files:|package size:")
  elif npm view "$name" version >/dev/null 2>&1; then
    echo "$name already published, skipped"
  else
    (cd "$pkg" && npm publish --ignore-scripts --access public)
  fi
done
