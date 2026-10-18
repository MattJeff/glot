#!/bin/sh
# glot installer: curl -fsSL https://raw.githubusercontent.com/MattJeff/glot/main/integrations/install.sh | sh
# Env: GLOT_VERSION (vX.Y.Z, default latest), GLOT_INSTALL_DIR (default ~/.local/bin),
#      GLOT_DOWNLOAD_BASE (mirror holding the assets + SHA256SUMS).
# The archive is verified against the release's SHA256SUMS asset, or, if the release has none,
# against the SHA-256 digest GitHub records for the asset. No checksum, no install.
set -eu

REPO=MattJeff/glot
VERSION=${GLOT_VERSION:-latest}
DIR=${GLOT_INSTALL_DIR:-$HOME/.local/bin}

err() { echo "glot-install: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "missing required command: $1"; }
need curl; need tar; need uname

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)               target=aarch64-apple-darwin ;;
  Darwin-x86_64)              target=x86_64-apple-darwin ;;
  Linux-aarch64|Linux-arm64)  target=aarch64-unknown-linux-gnu ;;
  Linux-x86_64)               target=x86_64-unknown-linux-gnu ;;
  *) err "no prebuilt binary for $(uname -s)-$(uname -m); use: cargo install --git https://github.com/$REPO glot-cli (Windows: npx glot-i18n)" ;;
esac
asset="glot-$target.tar.gz"

if [ "$VERSION" = latest ]; then
  VERSION=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')
  case "$VERSION" in v*) ;; *) err "could not resolve the latest release" ;; esac
fi
base=${GLOT_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$VERSION}

if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else err "need sha256sum or shasum to verify the download"; fi

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
echo "Downloading glot $VERSION ($target)"
curl -fsSL "$base/$asset" -o "$tmp/$asset" || err "download failed: $base/$asset"

if curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" 2>/dev/null; then
  expected=$(awk -v f="$asset" '$2 == f || $2 == "*"f { print $1 }' "$tmp/SHA256SUMS")
else
  # ponytail: flat-JSON scan of the release API; fine for GitHub's one-asset-per-object layout.
  expected=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/$VERSION" \
    | tr ',{}' '\n' | awk -v f="\"$asset\"" '
        /"name":/ { gsub(/[ \t]/, ""); hit = ($0 == "\"name\":" f) }
        hit && /"digest":/ { sub(/.*sha256:/, ""); sub(/".*/, ""); print; exit }')
fi
[ -n "${expected:-}" ] || err "no published SHA-256 for $asset; refusing to install"
actual=$(sha "$tmp/$asset")
[ "$actual" = "$expected" ] || err "checksum mismatch for $asset (expected $expected, got $actual)"

tar -xzf "$tmp/$asset" -C "$tmp" glot
mkdir -p "$DIR"
install -m 755 "$tmp/glot" "$DIR/glot" 2>/dev/null || { cp "$tmp/glot" "$DIR/glot"; chmod 755 "$DIR/glot"; }
echo "Installed $("$DIR/glot" --version) to $DIR/glot (sha256 verified)"
case ":$PATH:" in *":$DIR:"*) ;; *) echo "Add $DIR to your PATH: export PATH=\"$DIR:\$PATH\"" ;; esac
echo "Next: cd your-app && glot init"
