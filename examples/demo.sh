#!/usr/bin/env bash
# glot demo: from a hardcoded-English Next.js SaaS to a validated, multi-locale app.
#
#   examples/demo.sh                      # mock LLM, no API key needed, never touches demo-saas/
#   DEMO_PAUSE=1 examples/demo.sh         # wait for Enter between steps (live demos)
#   GLOT_REAL=1 ANTHROPIC_API_KEY=... examples/demo.sh   # real translations
#   GLOT=/path/to/glot KEEP=1 examples/demo.sh           # custom binary, keep the temp copy
#   NO_APP=1 examples/demo.sh             # skip pnpm install / dev server (no --visual, no fit)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/examples/demo-saas"
TARGETS="${TARGETS:-fr,de,ja}"
PORT="${PORT:-3200}"
BASE="http://localhost:$PORT"
ROUTES="/,/invoices/inv_1042,/settings/team"

if [[ -z "${GLOT:-}" ]]; then
  if [[ -x "$ROOT/target/integ/debug/glot" ]]; then GLOT="$ROOT/target/integ/debug/glot"
  elif command -v glot >/dev/null; then GLOT="$(command -v glot)"
  else echo "glot not found: cargo build -p glot-cli, or set GLOT=/path/to/glot" >&2; exit 1; fi
fi

bold=$'\e[1m'; dim=$'\e[2m'; cyan=$'\e[36m'; yellow=$'\e[33m'; green=$'\e[32m'; reset=$'\e[0m'
STEP=0
step()  { STEP=$((STEP + 1)); printf '\n%s━━ %d. %s%s\n' "$bold$cyan" "$STEP" "$1" "$reset"; [[ -n "${2:-}" ]] && printf '%s%s%s\n' "$dim" "$2" "$reset"; return 0; }
pause() { [[ "${DEMO_PAUSE:-0}" == 1 ]] && { read -rp "${dim}[Enter]${reset} " _ </dev/tty; }; return 0; }
say()   { printf '%s%s%s\n' "$dim" "$*" "$reset"; }
run()   { printf '%s$ glot %s%s\n' "$yellow" "$*" "$reset"; "$GLOT" "$@"; }
# A command exists if `glot --help` lists it (keeps the script usable on older builds).
has()   { "$GLOT" --help | grep -qE "^  $1 "; }
skip()  { say "(glot $1 is not available in this build, skipping)"; }

# Throwaway copy (git-tracked so we can show what glot rewrote): demo-saas stays hardcoded.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/glot-demo.XXXXXX")"
DEV_PID=""
cleanup() {
  [[ -n "$DEV_PID" ]] && kill "$DEV_PID" 2>/dev/null
  [[ "${KEEP:-0}" == 1 ]] || rm -rf "$WORK"
  return 0
}
trap cleanup EXIT
tar -C "$APP" --exclude node_modules --exclude .next --exclude .ready -cf - . | tar -C "$WORK" -xf -
cd "$WORK"
git init -q && git add -A && git -c user.name=demo -c user.email=demo@glot.dev commit -qm "hardcoded app"
printf '%sglot demo%s  binary: %s\n           app copy: %s\n' "$bold" "$reset" "$GLOT" "$WORK"

PROVIDER=mock
if [[ "${GLOT_REAL:-0}" == 1 ]]; then
  [[ -n "${ANTHROPIC_API_KEY:-}" ]] || { echo "GLOT_REAL=1 needs ANTHROPIC_API_KEY" >&2; exit 1; }
  PROVIDER=anthropic
fi

# ───────────────────────────────────────────────────────────────────────────────
step "glot init" "Detect the stack, write glot.toml + glossary + agent rules (CLAUDE.md, MCP, CI)."
pause
run init --yes --source en --targets "$TARGETS" --provider "$PROVIDER"

# ───────────────────────────────────────────────────────────────────────────────
step "glot extract" "Acme Invoices is 100% hardcoded English. glot finds every string, its UI role and a suggested key."
pause
{ run extract || true; } | head -20
say "… $("$GLOT" extract --json | grep -c '"suggested_key"') hardcoded strings in total"

# ───────────────────────────────────────────────────────────────────────────────
step "glot i18nize" "Rewrite the code to t() calls, name keys, turn concatenations into ICU plurals, then wire next-intl (proxy, provider, config)."
pause
I18NIZED=0
if has i18nize; then
  # Install next-intl only when the app will run (pnpm available); otherwise just print the command.
  INSTALL=--no-install
  [[ "${NO_APP:-0}" != 1 ]] && command -v pnpm >/dev/null && INSTALL=""
  run i18nize --yes --switcher $INSTALL
  I18NIZED=1
  echo
  say "What changed:"
  git --no-pager diff --stat | tail -1
  git --no-pager diff -U0 app/page.tsx | grep -E '^[-+] ' | head -8
  say "Runtime wired by i18nize: $(git ls-files --others --exclude-standard | grep -E '^(i18n/|proxy|components/LocaleSwitcher)' | tr '\n' ' ')"
  say "Source catalog: $(grep -c '": "' locales/en.json) strings in locales/en.json"
else
  skip i18nize
  say "Seeding the source catalog by hand instead (what i18nize would produce):"
  run add dashboard.overdue_banner "{count, plural, one {You have # overdue invoice} other {You have # overdue invoices}}" --note "Alert banner on the dashboard"
  run add invoices.reminder_sent "Reminder sent to {customer}" --note "Toast after clicking Send reminder"
  run add invoices.actions.mark_paid "Mark as paid" --note "Button, fixed width w-28"
  run add invoices.actions.archive "Archive" --note "Verb: button that archives an invoice"
  run add common.save "Save" --note "Verb: submit button of the new-invoice form"
fi

# The running app gives translate --visual and fit something to look at.
APP_UP=0
if [[ "$I18NIZED" == 1 && "${NO_APP:-0}" != 1 ]] && command -v pnpm >/dev/null; then
  say "pnpm install + next dev on :$PORT …"
  if pnpm install --silent --prefer-offline >/dev/null 2>&1; then
    pnpm exec next dev -p "$PORT" >"$WORK/.dev.log" 2>&1 &
    DEV_PID=$!
    for _ in $(seq 1 60); do curl -sf "$BASE/" >/dev/null 2>&1 && { APP_UP=1; break; }; sleep 1; done
  fi
  if [[ "$APP_UP" == 1 ]]; then
    for r in ${ROUTES//,/ }; do curl -sf "$BASE$r" >/dev/null || true; done # warm up the compiler
    printf '%s✓ app running at %s%s\n' "$green" "$BASE" "$reset"
  else
    say "(app did not start, see $WORK/.dev.log; continuing without --visual and fit)"
  fi
fi

# ───────────────────────────────────────────────────────────────────────────────
step "glot translate" "Only the delta is translated; every output goes through the deterministic validator. Provider: $PROVIDER."
pause
run translate --dry-run
if [[ "$APP_UP" == 1 ]] && "$GLOT" translate --help | grep -q -- --visual; then
  say "--visual: the LLM also sees a crop of where each key renders in the app."
  run translate --yes --visual --base-url "$BASE" --routes "$ROUTES"
else
  run translate --yes
fi

# ───────────────────────────────────────────────────────────────────────────────
step "glot check" "ICU syntax, placeholders, CLDR plural categories, glossary, length. CI-ready (text|json|sarif|github)."
pause
run check || true
run status

# ───────────────────────────────────────────────────────────────────────────────
step "glot fit" "German in a w-28 button? Capture each locale, shorten what overflows, re-render, repeat."
pause
if ! has fit; then skip fit
elif [[ "$APP_UP" != 1 ]]; then say "(needs the running app, skipping)"
else run fit --yes --base-url "$BASE" --routes "$ROUTES" --locales de --max-iterations 2 || say "(fit did not converge, see above)"
fi

# ───────────────────────────────────────────────────────────────────────────────
step "glot review" "Back-translate and explain: can a reviewer who doesn't speak Japanese trust it?"
pause
if has review; then
  { run review --locale ja || true; } | head -25
else skip review; fi

# ───────────────────────────────────────────────────────────────────────────────
step "glot glossary mine" "Find product terms (Invoice, Workspace…) translated inconsistently across the catalog."
pause
if "$GLOT" glossary --help | grep -qE '^  mine '; then
  { run glossary mine || true; } | head -20
else skip "glossary mine"; fi

# ───────────────────────────────────────────────────────────────────────────────
step "Summary"
LOCALE_FILES="$(ls locales 2>/dev/null | tr '\n' ' ' || true)"
cat <<EOF
  app copy     $WORK
  code         $(git --no-pager diff --shortstat HEAD 2>/dev/null | sed 's/^ //')
  catalogs     ${LOCALE_FILES:-none}
  lockfile     $([[ -f glot.lock ]] && echo "glot.lock ($(grep -c . glot.lock) lines)" || echo none)
  agent setup  CLAUDE.md, .mcp.json, .claude/skills/glot, .cursor/rules, .github/workflows/glot.yml

Next:
  claude mcp add glot -- glot mcp   # your coding agent keeps i18n in sync while you code
EOF
has dev && echo "  glot dev                          # in-app editing overlay (@glot/overlay)"
[[ "${KEEP:-0}" == 1 ]] && echo "Kept: cd $WORK && pnpm dev -p $PORT   then open $BASE/de"
exit 0
