#!/usr/bin/env bash
# End-to-end `glot assets` on this demo with the offline mock provider (no API key):
# i18nize -> translate (fr, de, ja, ar) -> render -> check, then a second run must change nothing.
# Usage: ./e2e.sh [out-dir]   (needs the `glot` binary on PATH, or GLOT=/path/to/glot)
set -euo pipefail
GLOT=${GLOT:-glot}
OUT=${1:-${TMPDIR:-/tmp}/glot-assets-demo}
rm -rf "$OUT" && mkdir -p "$OUT" && cp -R "$(dirname "$0")"/{glot.toml,package.json,messages,emails,og,assets} "$OUT"
cd "$OUT"

"$GLOT" assets --yes
# Real ja/ar copy (the mock only prefixes `⟦ja⟧`), validated like any human edit.
"$GLOT" set ja svg.onboarding.step_create "作成"
"$GLOT" set ja svg.onboarding.up_and_running_in "5分で使い始められます"
"$GLOT" set ja og.launch.ship_localized_apps_minutes "数か月ではなく数分で、多言語アプリを公開"
"$GLOT" set ar svg.onboarding.step_create "أنشئ"
"$GLOT" set ar svg.onboarding.up_and_running_in "جاهز للعمل في خمس دقائق"
"$GLOT" set ar og.launch.ship_localized_apps_minutes "أطلق تطبيقات مترجمة في دقائق، لا في أشهر"
"$GLOT" assets preview
"$GLOT" assets check

# Idempotence: nothing left to convert, translate or re-render.
before=$(find . -type f -not -path './.glot/batch.json' -exec shasum {} + | sort)
"$GLOT" assets --yes >/dev/null
after=$(find . -type f -not -path './.glot/batch.json' -exec shasum {} + | sort)
[ "$before" = "$after" ] || { echo "second run changed files"; diff <(echo "$before") <(echo "$after"); exit 1; }

for loc in en fr de ja ar; do
  test -s "public/og/$loc/launch.png" && test -s "public/img/$loc/onboarding.png" \
    && test -s "public/img/$loc/onboarding.svg" && test -s ".glot/emails/welcome/$loc.html" \
    && test -s ".glot/emails/invoice_paid/$loc.html"
done
grep -q 'dir="rtl"' .glot/emails/invoice_paid/ar.html
echo "OK: renders in $OUT/public/og, $OUT/public/img, $OUT/.glot/emails"
