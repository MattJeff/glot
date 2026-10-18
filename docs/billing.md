# Billing (glot cloud)

glot is MIT open source. **The CLI, the local MCP server, the GitHub Action and everything that runs on your machine are
never limited and never call billing code.** Billing only meters what the hosted cloud serves: CDN/OTA catalogs,
insights, remote MCP, on-demand locales and server-side translation with glot's own LLM key.

A server without `STRIPE_SECRET_KEY` (every self-hosted install) has billing off: `GET /api/billing` answers
`enabled: false`, and no quota applies.

Code: `crates/server/src/billing.rs`, migration `0080_billing.sql`, tests `crates/server/tests/billing.rs`,
pages `web/app/[locale]/pricing` and `web/app/[locale]/app/settings/billing`, setup `deploy/stripe-setup.sh`.

## Model

One billing account per user; it covers the projects that user **owns** (no organizations yet). Monthly counters
(`billing_usage`) reset on the 1st (UTC). Over a limit, the endpoint answers **402** `quota_exceeded` / `plan_required` /
`plan_limit` with a message that names the quota and links to `/app/settings/billing`.

| Monthly limit (default) | Free | Pro | Team | Enforced in |
|---|---|---|---|---|
| Cloud projects (owned) | 1 | 5 | 25 | `projects::create` |
| Seats | 1 | 3 | 15 | not enforced yet (no invite flow) |
| CDN/OTA requests | 50k | 1M | 10M | `cdn::serve` |
| Insights events | 10k | 250k | 2M | `insights::ingest` |
| Remote MCP calls (HTTP requests to `/mcp`) | 500 | 20k | 200k | `mcp_remote::guard` |
| On-demand locale jobs | 0 | 30 | 300 | `ondemand::gate` |
| Strings translated with glot's LLM key | 500 | 10k | 50k | `jobs::create`, `ondemand::gate` (check), `jobs::run` (count) |

- **BYOK is never counted.** "glot's key" = a project without its own API key on `anthropic`/`openai` without `base_url`
  (the job then uses the server's provider env var).
- **Usage-based**: monthly Pro/Team checkouts also add the metered price `glot_llm_strings` (Billing Meter
  `glot_llm_strings`, when `STRIPE_METER_EVENT` is set). Strings above the plan's allowance are then reported as
  meter events (`identifier = glot-job-<id>`, so a retried report is not billed twice) instead of being refused.
  Yearly plans stop at their allowance.
- Limits are overridable without a deploy of code: `GLOT_PLAN_LIMITS` (JSON per plan, `null` = unlimited).
  Prices live only in Stripe (lookup keys), the pricing page reads them from `GET /api/billing/plans`.

### Subscription state

| Stripe status | Plan applied |
|---|---|
| `active`, `trialing` | subscribed plan |
| `past_due` | subscribed plan until `grace_until` (= first `invoice.payment_failed` + `GLOT_BILLING_GRACE_DAYS`, default 7) |
| `unpaid`, `canceled`, `incomplete*`, none | Free |

Existing projects and data are never deleted on downgrade; only new projects and over-quota requests are refused.

## API

```
GET  /api/billing            -> {enabled, plan (in force), subscribed_plan, status, current_period_end, cancel_at_period_end,
                                 grace_until, metered, has_customer, limits: Limits, usage: {<limit key>: n}}
POST /api/billing/checkout   {plan: "pro"|"team", interval: "month"|"year"} -> {url}
       no subscription: Checkout Session (mode=subscription, automatic_tax if STRIPE_AUTOMATIC_TAX=1, tax id collection,
       promotion codes); live subscription: swaps the plan price (prorated) and returns the billing page URL.
POST /api/billing/portal     -> {url}   Customer Portal (payment method, invoices, tax ids, cancel at period end)
GET  /api/billing/invoices   -> [{id, number, status, total, currency, created, hosted_invoice_url, invoice_pdf}]
GET  /api/billing/plans      (public) -> {enabled, plans: {free|pro|team: Limits}, prices: {<lookup_key>: {...}}}
POST /api/billing/webhook    (Stripe) Stripe-Signature verified (HMAC-SHA256, 5 min tolerance)
```

Webhook events handled: `checkout.session.completed` (links the customer), `customer.subscription.created|updated|deleted`
(plan from the price `metadata.glot_plan` or lookup key, period end from the items, out-of-order events ignored by
`event.created`, a deletion of a replaced subscription ignored), `invoice.paid` (clears grace), `invoice.payment_failed`
(starts grace). Idempotency: the event id is inserted in `stripe_events` in the same transaction as its effects; a
duplicate gets 200 without effect, a failure rolls back so Stripe's retry re-applies it.
API version pinned: `Stripe-Version: 2026-09-30.endive` (requests and the webhook endpoint created by the script).

## Stripe keys: use restricted keys

Never put the full-access `sk_live_` key on the server. Create two **restricted keys** (Dashboard > Developers >
API keys > Create restricted key), first in a sandbox/test mode, then in live with the same permissions:

**`glot-server` (runtime, goes in `deploy/.env` as `STRIPE_SECRET_KEY`)**

| Resource | Permission | Used by |
|---|---|---|
| Customers | Write | create the customer at first checkout/portal |
| Checkout Sessions | Write | `POST /api/billing/checkout` |
| Customer portal | Write | `POST /api/billing/portal` |
| Subscriptions | Write | plan change (price swap) |
| Prices | Read | lookup keys (checkout, pricing page) |
| Invoices | Read | invoice list |
| Billing meter events (Meter events) | Write | usage-based overage (only if `STRIPE_METER_EVENT`) |
| everything else | None | |

**`glot-setup` (used once by `deploy/stripe-setup.sh`, from your laptop; revoke or keep offline afterwards)**

| Resource | Permission |
|---|---|
| Products | Write |
| Prices | Write |
| Billing meters | Write |
| Customer portal (configurations) | Write |
| Webhook endpoints | Write |

If a call is missing a permission, Stripe's error names it; the server logs it (`stripe request failed`). Optionally
restrict the runtime key to the VPS IP.

## Setup

```sh
# 1. Test mode (default). For a test webhook pointing at your machine use the Stripe CLI instead:
#    stripe listen --forward-to localhost:8790/api/billing/webhook  (prints a whsec_ for STRIPE_WEBHOOK_SECRET)
STRIPE_SECRET_KEY=rk_test_... deploy/stripe-setup.sh --webhook-url https://<staging>/api/billing/webhook

# Prices (minor units, EUR, tax-exclusive) are env-overridable:
PRO_MONTH=1900 PRO_YEAR=19000 TEAM_MONTH=7900 TEAM_YEAR=79000 LLM_PER_STRING=0.15 STRIPE_SECRET_KEY=... deploy/stripe-setup.sh
```

The script is idempotent: fixed product ids (`glot_pro`, `glot_team`, `glot_llm`), prices found by lookup key
(`glot_{pro,team}_{month,year}`, `glot_llm_strings`; a changed amount creates a new price and transfers the lookup key),
meter found by event name, portal configuration by `metadata.glot=1`, webhook endpoint by URL. It refuses a live key
without `--live`, and `--live` asks you to type a confirmation phrase.

Local tests: `docker run -d --rm -p 12111:12111 stripe/stripe-mock` then
`cargo test -p glot-server --test billing` (the webhook part only needs Postgres).

## Going live (checklist)

1. Stripe account (FR, EUR) activated; **Stripe Tax**: set the origin address, add the registrations you need (France VAT;
   EU OSS once you sell B2C cross-border), set the default tax code to SaaS business use (`txcd_10103001`).
2. Settings > Billing > Customer emails: enable failed-payment emails and Smart Retries (the dunning window should be
   longer than `GLOT_BILLING_GRACE_DAYS`), and "cancel subscription if all retries fail".
3. Create the two restricted live keys above.
4. On your laptop: `STRIPE_SECRET_KEY=<glot-setup rk_live> deploy/stripe-setup.sh --live` (type the phrase); note
   `STRIPE_WEBHOOK_SECRET` (shown only at creation) and `STRIPE_PORTAL_CONFIGURATION`.
5. On the VPS, `deploy/.env`: `STRIPE_SECRET_KEY=<glot-server rk_live>`, `STRIPE_WEBHOOK_SECRET`,
   `STRIPE_PORTAL_CONFIGURATION`, `STRIPE_METER_EVENT=glot_llm_strings`, `STRIPE_AUTOMATIC_TAX=1`; then `deploy/deploy.sh`.
   The log line `billing enabled live=true` confirms it.
6. Smoke test: subscribe your own account with a real card on Pro monthly, check `/app/settings/billing` shows Pro,
   the invoice appears, the webhook endpoint shows 200s in Workbench; cancel in the portal and refund from the dashboard.
7. Revoke or archive the `glot-setup` key.
