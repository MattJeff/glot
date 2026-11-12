# Local SEO — search-facing content per page × market

Status: first working version (server module, API, dashboard page, docs, tests). November 2026.

## Problem

Translation tools translate keywords. People do not search with translated keywords: a French "Recadrez vos vidéos"
becomes "Recorta tus vídeos", while Madrid types "recortar vídeo online" and Mexico City "cortar video en línea".
The page ranks for words nobody types, and the customer concludes "the Spanish site does not work".

## What exists

- **Weglot**: translates meta title / description, image alts and slugs, adds hreflang. Its own multilingual SEO page
  tells customers to research keywords themselves; the editor lets them type the result in.
  (https://weglot.dev/multilingual-seo, https://rankmath.com/kb/wordpress-multilingual-seo-with-rankmath-weglot,
  https://www.godaddy.com/garage/how-to-improve-international-seo/)
- **Lokalise and others**: guides that describe "SEO localization" as keyword research + adaptation done by people.
  (https://lokalise.com/blog/seo-localization/)
- **Keyword tools** (Semrush, Ahrefs, Sistrix…): per-country keyword databases, paid, not connected to the
  translation layer: someone copies keywords from one tool into the other.
- **glot today**: glot Edge translates title / description / og / JSON-LD, emits hreflang, sitemaps, IndexNow and an
  international SEO report (`crates/server/src/edge/seo_report.rs`) that says "effectively indexed: unknown until
  Search Console is connected". `glot seo keywords` (CLI, `crates/llm/src/seo.rs`) asks the LLM to guess search terms
  for catalog metadata keys: no demand data, no review flow, no measurement.

Not verified: whether Smartling, Phrase or Surfer ship a per-market title generator today (the search did not
surface one; this document does not claim they do not).

## What is new

1. The loop is closed inside the translation layer: **real demand per country → proposal → validators → human
   approval → served by Edge → measured in the same Search Console**.
2. A market is a **locale**: `es-ES` and `es-MX` have their own demand, proposals and approvals.
3. **Validators with teeth**: a proposal cannot claim what the page does not say ("gratis" is searched; the page is
   not free; the title is refused), cannot stuff, cannot drop a brand term, cannot duplicate another page.
4. **No invented data**: no keyword, no volume, no metric is ever estimated. No data → the market says so.

## Architecture

```
POST /local-seo/scan
  pages   ← raw HTML of the source site (SSRF-safe fetch, anonymous) or content sent in the request
  current ← the live localized page (glot Edge domain) or the stored translations
  demand  ← DemandSource[]: Manual (customer's list), SearchConsole (customer's OAuth, read-only)
  propose ← project LLM (only when demand terms exist) → validate → one retry with the objections
  store   ← local_seo_overrides (status proposed | auto | needs_demand)
approve / edit / reject  → status approved | rejected, bump edge_catalog_version
serve   ← local_seo::rewrite(db, project, locale, path, html)   (glot Edge)
          GET /cdn/{slug}/local-seo/{locale}.json                (website tag)
measure ← POST /local-seo/metrics/refresh → local_seo_metrics, compared 28 days before / after applied_at
```

Code: `crates/server/src/local_seo/{mod,validate,demand,html}.rs`. `validate.rs` and `html.rs` are pure.

### The hook for glot Edge (owned by another agent, not edited here)

In `edge/proxy.rs`, on the translated HTML just before the response is built:

```rust
let html = crate::local_seo::rewrite(&state.db, p.id, locale, path, html).await;
```

No approved override → the page is returned untouched. Decisions bump `edge_catalog_version`, the version Edge's
caches and snapshots already key on. The website tag would read `/cdn/{slug}/local-seo/{locale}.json` and set
`document.title`, the description and the H1 (weaker than server-side: crawlers that do not run JavaScript miss it).

### Resolution order (`validate::resolve`)

1. the market's own row if served (`approved` / `auto`);
2. nothing if the market's own row is `rejected`;
3. the bare-language row (`es` for `es-MX`) if served;
4. nothing. Never a sibling region, never a draft.

### Demand sources

| Source | Free | Allowed | State |
|---|---|---|---|
| Customer's keyword list | yes | their data | built |
| Google Search Console API (`searchanalytics.query`, country + page filters), customer's OAuth, `webmasters.readonly` | yes | official API | built; needs the founder's OAuth client |
| Bing Webmaster API (`GetQueryStats`) | yes | official | adapter to write |
| IndexNow | yes | official | already in glot Edge; a ping, not a demand source |
| Customer's analytics (glot Insights) | yes | their data | says which countries visit, not which words; not used for keywords |
| Google Trends API | — | alpha, application only (https://developers.google.com/search/blog/2025/07/trends-api) | not available |
| Google Autocomplete (`suggestqueries`) | — | unpublished; Google restricted it in 2015 (https://developers.google.com/search/blog/2015/07/update-on-autocomplete-api) | refused |
| Google Ads Keyword Planner API | free with an Ads account | restricted to ads use, developer token review | not used |
| DataForSEO, Semrush, Ahrefs, Keywords Everywhere, SerpApi | paid | with the customer's key | adapters behind `DemandSource`, none written or called |

Honest limit: Search Console only returns queries the site already appears for in that country. A brand-new market
returns nothing; the customer's own list is the cold-start path. This is the main product risk (see Risks).

## Data model (migration 0186)

`local_seo_settings` (mode, site_url, who consented to auto) · `local_seo_pages` (scanned source content,
fingerprint) · `local_seo_demand` (term, source, impressions/clicks/position, nullable) · `local_seo_overrides`
(one row per project × locale × path: current, proposed, edited, terms, rationale, issues, status, applied_at) ·
`local_seo_connections` (Search Console property, encrypted refresh token) · `local_seo_metrics` (daily).

## Privacy and security

- Sent to the LLM: public page copy (title, description, headings, 2 000-character excerpt), aggregated search
  queries, glossary terms. No visitor data exists in this module.
- Pages are fetched anonymously with the existing SSRF guard (`review_live::probe::fetch_with` through
  `edge::sitemap::fetch`): public addresses only, pinned DNS, bounded body.
- Search Console: read-only scope, refresh token AES-256-GCM encrypted, never returned; OAuth `state` is an HMAC over
  project + user + 10-minute expiry; the callback requires the same logged-in admin.
- Search Console queries are already aggregated and thresholded by Google.
- Text written into HTML is escaped (lol_html text content; `</` neutralized in JSON-LD).
- Consent: default mode `review`; `auto` is an admin setting recorded with its author, and only proposals with zero
  issues are auto-applied.

## Pricing idea

Audit (what each market shows today) free on every plan: it costs no LLM call. Proposals count against the LLM
quota like translations. Search Console connection, auto mode and before/after tracking on Pro and above.
Not enforced yet beyond `billing::check_llm`.

## Phases

1. **Done**: scan, audit, manual + Search Console demand, proposals, validators, approval, read hook, public JSON,
   metrics storage and before/after view, dashboard, docs.
2. Edge calls the hook; website tag applies the JSON; background job for scans larger than 10 pages; weekly metric
   refresh.
3. Bing Webmaster adapter; paid adapters with the customer's key; image alts; visible FAQ block (needs a slot in
   the customer's layout); feed slug suggestions into Edge's translated slugs with redirects.
4. Entailment judge for claims (today: lexicon + figures); per-query attribution of the before/after change.

## Risks

- **Cold start**: no impressions in a new market → no Search Console terms. Mitigated only by the customer's list
  until a free, terms-compliant keyword source exists. This limits the "zero configuration" promise.
- **Attribution**: before/after is a comparison, not an experiment; seasonality and Google updates move numbers.
  The dashboard shows the two windows and claims no causality.
- **Claims check is lexical**: it catches free / best / official / guaranteed / discounts / figures in twelve
  languages, not subtler inventions. Human review is the default for that reason.
- **Engine limits are guidelines** (pixels, not characters); Baidu / Naver / Yandex budgets are conservative
  defaults.
- **Title rewrites by Google**: Google may ignore a title; measurement is the only judge.
