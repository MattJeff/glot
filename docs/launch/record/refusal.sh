#!/usr/bin/env bash
# Refusal demo: a real `glot translate` against an LLM that gets three strings wrong on the first pass.
#   docs/launch/record/refusal.sh WORK                 scripted LLM (refusal-llm.mjs), offline
#   UPSTREAM=https://api.anthropic.com/v1 UPSTREAM_KEY=$ANTHROPIC_API_KEY MODEL=claude-haiku-5-5 \
#     docs/launch/record/refusal.sh WORK               same first pass, retries by the real model
# Then: python3 docs/launch/record/refusal-term.py WORK && node docs/launch/record/term.mjs WORK
# ponytail: the pixel budget is hand-written (Geist 14px, 120px, one line), as `glot measure` would record it.
set -euo pipefail
W=$(mkdir -p "$1" && cd "$1" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
GLOT=${GLOT:-$ROOT/target/integ/debug/glot}
PORT=${PORT:-18799}
P="$W/app"
rm -rf "$P" && mkdir -p "$P/messages" "$P/.glot/fonts"
cp "$ROOT/crates/core/tests/fixtures/Geist-Regular.ttf" "$P/.glot/fonts/"
cat > "$P/glot.toml" <<EOF
source_locale = "en"
target_locales = ["pl", "fr", "de"]
files = ["messages/{locale}.json"]
[llm]
provider = "openai-compatible"
base_url = "http://127.0.0.1:$PORT"
model = "${MODEL:-scripted-llm}"
EOF
cat > "$P/messages/en.json" <<'EOF'
{
  "home": { "welcome": "Welcome back, {name}!" },
  "inbox": { "unread": "{count, plural, one {# unread message} other {# unread messages}}" },
  "invoice": { "remind": "Send reminder" }
}
EOF
cat > "$P/.glot/layout.json" <<'EOF'
{ "invoice.remind": { "max_width_px": 120, "max_lines": 1,
  "font": { "family": "Geist", "postscript": "Geist-Regular", "size_px": 14, "weight": 400 } } }
EOF
node "$HERE/refusal-llm.mjs" "$PORT" 2> "$W/llm.log" &
trap 'kill $! 2>/dev/null' EXIT
sleep 0.5
cd "$P"
{
  echo '$ cat messages/en.json' && cat messages/en.json
  echo '$ glot translate --yes' && RUST_LOG=error NO_COLOR=1 "$GLOT" translate --yes 2>&1
  echo '$ glot check' && RUST_LOG=error NO_COLOR=1 "$GLOT" check 2>&1
} | tee "$W/refusal.log"
