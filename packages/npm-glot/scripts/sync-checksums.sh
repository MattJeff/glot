#!/bin/sh
# Pin the release's SHA-256 digests into checksums.json (run before `npm publish`).
# Usage: scripts/sync-checksums.sh [vX.Y.Z]   (default: v$(package.json version))
set -eu
cd "$(dirname "$0")/.."
tag=${1:-v$(node -p 'require("./package.json").version')}
gh release view "$tag" -R MattJeff/glot --json assets \
  --jq '[.assets[] | select(.name | test("^glot-.*\\.(tar\\.gz|zip)$")) | {key: .name, value: (.digest | sub("^sha256:"; ""))}] | from_entries' \
  > checksums.json
cat checksums.json
