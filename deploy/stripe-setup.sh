#!/usr/bin/env bash
# Creates glot's Stripe catalog, idempotently (re-run = no duplicates): products, prices (found by lookup_key),
# the LLM usage meter + metered price, a Customer Portal configuration and the webhook endpoint.
# Prints the variables for deploy/.env. See docs/billing.md.
#
#   STRIPE_SECRET_KEY=rk_test_... deploy/stripe-setup.sh                 # test mode (default)
#   STRIPE_SECRET_KEY=rk_live_... deploy/stripe-setup.sh --live          # live: asks for explicit confirmation
#   options: --webhook-url URL (default https://api.$GLOT_DOMAIN/api/billing/webhook) --return-url URL
#
# Prices are minor units of CURRENCY (default eur), tax-exclusive; override with env:
#   PRO_MONTH=1900 PRO_YEAR=19000 TEAM_MONTH=7900 TEAM_YEAR=79000 LLM_PER_STRING=0.15 (cents per string)
# Changing an amount and re-running creates a new price and moves the lookup key to it (new checkouts use it;
# existing subscribers keep their price until you migrate them in the dashboard).
# STRIPE_API_BASE=http://localhost:12111 runs it against stripe-mock.
set -euo pipefail

VERSION=2026-09-30.endive
LIVE=0
DOMAIN="${GLOT_DOMAIN:-glotlang.com}"
WEBHOOK_URL="https://api.$DOMAIN/api/billing/webhook"
RETURN_URL="https://app.$DOMAIN/app/settings/billing"
while [ $# -gt 0 ]; do
  case "$1" in
    --live) LIVE=1 ;;
    --webhook-url) WEBHOOK_URL="$2"; shift ;;
    --return-url) RETURN_URL="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

KEY="${STRIPE_SECRET_KEY:?set STRIPE_SECRET_KEY (a restricted rk_ key, see docs/billing.md)}"
API="${STRIPE_API_BASE:-https://api.stripe.com}"
CURRENCY="${CURRENCY:-eur}"
PRO_MONTH="${PRO_MONTH:-1900}" PRO_YEAR="${PRO_YEAR:-19000}"
TEAM_MONTH="${TEAM_MONTH:-7900}" TEAM_YEAR="${TEAM_YEAR:-79000}"
LLM_PER_STRING="${LLM_PER_STRING:-0.15}"
METER_EVENT=glot_llm_strings

case "$KEY" in
  sk_live_*|rk_live_*) MODE=live ;;
  *) MODE=test ;;
esac
if [ "$MODE" = live ] && [ "$LIVE" != 1 ]; then
  echo "refusing: this is a LIVE key. Test mode is the default; pass --live to write to live mode." >&2; exit 1
fi
if [ "$LIVE" = 1 ]; then
  [ "$MODE" = live ] || { echo "--live needs a live key (sk_live_/rk_live_)" >&2; exit 1; }
  cat <<EOF
About to create LIVE Stripe objects (real money):
  products glot_pro, glot_team, glot_llm; prices in $CURRENCY: pro $PRO_MONTH/mo $PRO_YEAR/yr, team $TEAM_MONTH/mo $TEAM_YEAR/yr,
  LLM $LLM_PER_STRING cents/string (meter $METER_EVENT); portal configuration; webhook -> $WEBHOOK_URL
EOF
  read -r -p "Type 'create live billing' to continue: " answer
  [ "$answer" = "create live billing" ] || { echo "aborted"; exit 1; }
fi

# s METHOD PATH [curl -d args...] -> JSON on stdout; exits on Stripe errors except resource_missing (404).
s() {
  local method="$1" path="$2"; shift 2
  local out code
  out=$(curl -sS -w '\n%{http_code}' -u "$KEY:" -H "Stripe-Version: $VERSION" -X "$method" "$API$path" "$@")
  code="${out##*$'\n'}"; out="${out%$'\n'*}"
  if [ "$code" -ge 400 ] && [ "$code" != 404 ]; then
    echo "Stripe $method $path -> $code: $out" >&2; exit 1
  fi
  printf '%s' "$out"
}
# j EXPR <json  : python expression over `d` (the parsed JSON); prints "" for None.
j() { python3 -c 'import json,sys; d=json.load(sys.stdin); v=eval(sys.argv[1]); print("" if v is None else v)' "$1"; }

product() { # id name plan
  if [ -z "$(s GET "/v1/products/$1" | j 'd.get("id") if d.get("object")=="product" else None')" ]; then
    s POST /v1/products -d id="$1" -d name="$2" -d "metadata[glot_plan]=$3" -d tax_code=txcd_10103001 >/dev/null
    echo "created product $1" >&2
  fi
}

price() { # lookup_key product amount interval plan [extra curl args...]; reuses the active price when the amount matches
  local lk="$1" prod="$2" amount="$3" interval="$4" plan="$5"; shift 5
  local field=unit_amount; case "$amount" in *.*) field=unit_amount_decimal ;; esac
  local cur
  cur=$(s GET /v1/prices -G -d "lookup_keys[]=$lk" -d active=true -d limit=1 \
        | j "next((p['id'] for p in d['data'] if str(p.get('$field')).rstrip('0').rstrip('.')=='$amount'.rstrip('0').rstrip('.') and p.get('currency')=='$CURRENCY'), None)")
  if [ -n "$cur" ]; then echo "$cur"; return; fi
  s POST /v1/prices -d lookup_key="$lk" -d transfer_lookup_key=true -d product="$prod" -d currency="$CURRENCY" \
    -d "$field=$amount" -d "recurring[interval]=$interval" -d tax_behavior=exclusive -d "metadata[glot_plan]=$plan" "$@" \
    | j 'd["id"]'
  echo "created price $lk" >&2
}

product glot_pro "glot Pro" pro
product glot_team "glot Team" team
product glot_llm "glot LLM translations (glot's key)" llm
price glot_pro_month glot_pro "$PRO_MONTH" month pro >/dev/null
price glot_pro_year glot_pro "$PRO_YEAR" year pro >/dev/null
price glot_team_month glot_team "$TEAM_MONTH" month team >/dev/null
price glot_team_year glot_team "$TEAM_YEAR" year team >/dev/null

METER=$(s GET /v1/billing/meters -G -d status=active -d limit=100 \
        | j "next((m['id'] for m in d['data'] if m.get('event_name')=='$METER_EVENT'), None)")
if [ -z "$METER" ]; then
  METER=$(s POST /v1/billing/meters -d display_name="glot LLM strings" -d event_name="$METER_EVENT" \
    -d "default_aggregation[formula]=sum" -d "customer_mapping[type]=by_id" \
    -d "customer_mapping[event_payload_key]=stripe_customer_id" -d "value_settings[event_payload_key]=value" | j 'd["id"]')
  echo "created meter $METER" >&2
fi
price glot_llm_strings glot_llm "$LLM_PER_STRING" month llm \
  -d "recurring[usage_type]=metered" -d "recurring[meter]=$METER" >/dev/null

PORTAL=$(s GET /v1/billing_portal/configurations -G -d active=true -d limit=100 \
         | j "next((c['id'] for c in d['data'] if (c.get('metadata') or {}).get('glot')=='1'), None)")
if [ -z "$PORTAL" ]; then
  PORTAL=$(s POST /v1/billing_portal/configurations -d "metadata[glot]=1" -d default_return_url="$RETURN_URL" \
    -d "business_profile[headline]=glot cloud billing" \
    -d "features[invoice_history][enabled]=true" -d "features[payment_method_update][enabled]=true" \
    -d "features[customer_update][enabled]=true" -d "features[customer_update][allowed_updates][]=email" \
    -d "features[customer_update][allowed_updates][]=address" -d "features[customer_update][allowed_updates][]=name" \
    -d "features[customer_update][allowed_updates][]=tax_id" \
    -d "features[subscription_cancel][enabled]=true" -d "features[subscription_cancel][mode]=at_period_end" | j 'd["id"]')
  echo "created portal configuration $PORTAL" >&2
fi

HOOK_SECRET="(unchanged: reveal it in Workbench > Webhooks, or roll it there)"
HOOK=$(s GET /v1/webhook_endpoints -G -d limit=100 | j "next((w['id'] for w in d['data'] if w.get('url')=='$WEBHOOK_URL'), None)")
if [ -z "$HOOK" ]; then
  out=$(s POST /v1/webhook_endpoints -d url="$WEBHOOK_URL" -d api_version="$VERSION" -d description="glot billing" \
    -d "enabled_events[]=checkout.session.completed" -d "enabled_events[]=customer.subscription.created" \
    -d "enabled_events[]=customer.subscription.updated" -d "enabled_events[]=customer.subscription.deleted" \
    -d "enabled_events[]=invoice.paid" -d "enabled_events[]=invoice.payment_failed")
  HOOK=$(printf '%s' "$out" | j 'd["id"]'); HOOK_SECRET=$(printf '%s' "$out" | j 'd.get("secret")')
  echo "created webhook endpoint $HOOK" >&2
fi

cat <<EOF

# ---- deploy/.env ($MODE mode) ----
STRIPE_SECRET_KEY=<runtime restricted key, NOT the setup key: see docs/billing.md>
STRIPE_WEBHOOK_SECRET=$HOOK_SECRET
STRIPE_PORTAL_CONFIGURATION=$PORTAL
STRIPE_METER_EVENT=$METER_EVENT
STRIPE_AUTOMATIC_TAX=1   # only once Stripe Tax is set up (origin address + registrations), else 0
# webhook endpoint: $HOOK -> $WEBHOOK_URL
EOF
