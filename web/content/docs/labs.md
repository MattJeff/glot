# Labs

Labs are the parts of glot outside the core loop (`init` → `i18nize` → `translate` → `check`, with pixel budgets and the MCP server). They work and are tested, but they are younger, less used, and their flags may change between minor versions. In the CLI they live under `glot labs <command>`; the old spelling (`glot seo`, `glot db`…) still works.

| | What it does | Where |
|---|---|---|
| Plural coverage | Every plural/select branch per locale, measured against the pixel budget, optionally rendered | `glot labs coverage` |
| Render safety | Glyphs your webfont lacks per locale, RTL bidi bugs, no-break spaces, quotes, digits; `--fix` | `glot labs render` |
| Review | Back-translation with explanations; `--stale` sorts cosmetic from real source changes | `glot labs review` |
| Register variants | Formal/informal catalogs generated from the base locale | `[variants]` in `glot.toml` |
| Task parity | An agent runs the same tasks in every locale and compares success | `glot labs parity` |
| Synthetic user panel | A native persona per market: 5-second test, CTA comprehension, trust, non-native wording → localization debt | `glot labs panel` |
| Cultural audit | US-centric code, missing legal pages, idioms and tone per market; a readiness score and transcreated suggestions | `glot labs culture` |
| SEO | hreflang/canonical/metadata audit per locale, localized keywords | `glot labs seo` |
| App Store / Play listings | fastlane metadata per country: keyword research, hard limits validated, release notes from git | `glot labs store` |
| Database content | Translates rows into a shadow table, read back by your ORM | `glot labs db`, `@glot-i18n/db` |
| Emails, SVG, OG images | Same pipeline as the UI, rendered per locale | `glot labs assets` |
| Unused keys | Source keys never referenced in code (`--prune`) | `glot labs unused` |
| Which language next? | Ranks languages by visitor demand, market data, exact cost and technical effort; `--add` adds the winner | `glot labs markets`, dashboard **Insights** |
| Distill | A free local model in your project's voice: reviewed pairs as a dataset, retrieval, eval vs cloud | `glot labs distill` |
| Insights, experiments, feedback, on-demand translation | Growth features of the [self-hosted server](self-hosting.md) | dashboard, **Labs** section |

## Plural coverage (`glot labs coverage`)

A page shows `5 plików`; nobody ever looks at `3 pliki` or `22 pliki`. `glot coverage` enumerates every branch of every plural / select / selectordinal message (ICU and MF2), per locale: each CLDR category of the locale with a real witness number (`pl`: 1, 3, 11, 1.5 · `ar`: 0, 1, 2, 3, 11, 100), each explicit `=0`, each select key, and their combinations when nested. Each branch is rendered as text and measured against the key's pixel budget (`.glot/layout.json` from `glot measure`, no browser).

```sh
glot coverage                                   # table: locale × category, branches, seen in a browser, failing, too wide
glot coverage --render --base-url http://localhost:3000 --routes /   # mount every branch on the app page
glot coverage --locale pl,ar --json
```

`--render` builds a test page per locale (one box per branch with the measured width, lines and font) and mounts it on your app's page so its CSS and webfonts apply: no instrumentation. visual-qa (`--harness`) reports overflow and truncation per branch; results are kept in `.glot/coverage.json` per branch text, so an edited translation counts as unseen again. The report reads like "pl few: 3 of 8 seen in a browser (37 %), 1 failing". Exit 1 on any failing or too-wide branch.

## Render safety (`glot labs render`)

A translation can be valid and still render wrong: Japanese drawn in a system font because the webfont only ships Latin, `%@ %d دولة` read in the wrong order once `%@` is a Latin name, `Copié !` with the `!` alone on the next line. `glot labs render` finds these per locale without a browser, from the catalogs and the font files.

```sh
glot labs render                   # table, exit 1 on errors
glot labs render --locale ar,ja --json
glot labs render --font public/fonts/Inter.woff2   # or a glob, or a family name
glot labs render --fix             # apply the fixable findings (each one through the validator)
```

- **Glyphs.** Fonts come from `[measure] fonts = ["public/fonts/*.woff2"]` in `glot.toml`, else every `.woff2`/`.ttf`/`.otf`/`.ttc` in the repo (`public/`, `.glot/fonts/`, `.next/static/media/` for next/font after a build), grouped by family; icon fonts are skipped. glot reads each font's `cmap` (WOFF2 decompressed, no glyph rebuild) and lists, per font × locale, the keys that fall back, the missing characters, their scripts, the likely platform fallback, and the subset to add: a `unicode-range` and its approximate woff2 size (`ja: 132 keys fall back to Hiragino Sans / Noto Sans CJK… · add a subset ≈ 79 KB, unicode-range: U+3000-303F, U+3040-30FF, U+4E00-9FFF`). U+FFFD (an encoding error) is an error; private-use characters are a warning. Emoji are left to the OS emoji font.
- **Bidi** (`ar`, `he`, `fa`, `ur`…). glot runs the Unicode Bidi Algorithm on a sample of the message (`%@`/`{name}` = a Latin value, `%d` = a number) and compares it with the same message where each text placeholder is isolated. A difference is an error: the value merges with a neighbouring number or punctuation and the line reorders. Fix: FSI/PDI (U+2068/U+2069) around the placeholder, like `<bdi>`. A string starting with a text placeholder or a Latin word is laid out LTR wherever the base direction comes from the first strong character (iOS/Android natural direction, `dir="auto"`): an error for native catalogs, a warning on the web. Fix: isolation, or a leading U+200F. Brackets typed in visual order (`)…(`) are an error (fix: swap them), unbalanced brackets a warning.
- **Typography** (warnings). French no-break spaces before `; : ! ?` and inside `« »` (U+202F, U+00A0 before `:`; not before `! ? ;` in `fr-CA`). A no-break space between a number and its unit (`10 €`, `42 %`) and inside `10 000` in space-grouping locales. Straight or English quotes where the locale uses `« »`, `„ “`, `「」`. A decimal separator copied from the source (`4.5` in `de`, only when the source has the same number, versions excluded). Digit systems that differ from the rest of the locale (Latin `5` among Arabic-Indic `٣`), reported only.

`--fix` rewrites only the target locales, through `set_translation`: a fix the validator refuses is not written, and machine-translated entries keep their provenance in `glot.lock`. Untranslated entries and source strings flagged by `glot check` (Swift code pasted into the catalog) are skipped. MCP: `render_check` returns the same report, each fixable finding with `fixed`, the value to pass to `set_translation`.

On a 13-locale iOS app (`.strings`, no webfont: system fonts, so no glyph gaps), it found 37 French spacing issues, all applied by `--fix` with `glot check` still green; the Arabic strings had no bidi bug (their placeholders sit between Arabic words or before `؟`). Against SF Pro (`--font /System/Library/Fonts/SFNS.ttf`), 132 keys each in `ar`, `ja`, `ko`, `th` and 131 in `hi` fall back to another font. On this site (Geist via next/font, Latin subset only), the Arabic and Japanese pages draw 650 keys with a fallback font, and every locale falls back for `⌘` and `≈`.

## Register variants (`[variants]`)

Formal vs informal is not a separate language, but it is a separate catalog. Declare variants per base locale:

```toml
[variants]
fr = ["formal", "informal"]   # vous / tu
de = ["sie", "du"]
en = ["plain"]                # plain language (accessibility)
ja = ["keigo"]
```

They are stored as BCP 47 private-use locales, `messages/fr-x-informal.json`. `glot translate` generates them **from the base locale** (`fr`, not `en`) after the base, with the register in the prompt, so both share terminology; they are tracked in `glot.lock` against the base text (a base change makes the variant stale) and `glot check` validates them against the base (same placeholders, plural categories, tags). At runtime it's just another locale: pick it from a user preference, e.g. with next-intl set the locale cookie (`NEXT_LOCALE=fr-x-informal`) when the user chooses "tutoiement", and add the variant tags to your `locales` list.

## Task parity (`glot labs parity`)

Users and agents complete fewer tasks in non-English UIs. `glot parity` measures it for your app: an agent (Claude Sonnet 5.5, screenshot + reduced accessibility tree, click/type/select/scroll) gets each goal **in the tested language** (translated once, cached in `.glot/parity/goals.json`) and tries to complete it in every locale.

```yaml
# glot.tasks.yaml — success conditions use catalog KEYS, so they hold in every language
- id: send_reminder
  goal: Invoice INV-1042 is overdue. Send the customer a payment reminder.
  start: /invoices
  success: { text_visible_key: invoice_actions.reminder_sent }   # and/or url_matches, selector
  script:                                                        # only for --provider mock
    - click: { selector: "a[href='/invoices/inv_1042']" }
    - click: invoice_actions.send_reminder_button               # found by its translated label
```

```sh
cd packages/agent-qa && pnpm install && cd -
glot parity --base-url http://localhost:3000 --locales en,fr,de,ja,ar [--url-pattern "{route}?lang={locale}"] [--locale-cookie NEXT_LOCALE] [--min-parity 90]
glot parity --provider mock      # scripted, deterministic, no tokens (CI)
```

Writes `.glot/parity/report.{md,json}` and a screenshot per step: a task × locale matrix (success, steps, time), a parity score per locale (share of the tasks the source locale completes), and findings tied to keys — truncated label, control not found, ambiguous label, broken RTL, untranslated text — with a fix (`glot set fr invoice_actions.send_reminder_button "Relancer"` when the agent looked for "Relancer"). Run it against a production build: dev servers recompile mid-run. The report stays local (no upload endpoint); post `report.md` as a PR comment. Example tasks: `packages/agent-qa/examples/demo-saas.tasks.yaml`.

## Synthetic user panel (`glot labs panel`)

A UX study in 12 languages for about a dollar. For each route × locale, `packages/agent-qa` captures what a visitor sees first (screenshot of the viewport), the page text and its calls to action (each matched to its catalog key). A synthetic native persona per locale then answers, in structured JSON: a **5-second test** ("what does this product offer?"), **what each CTA does** if clicked, **trust** (1–5 + reason), wording that **reads translated** (exact quote, the glot key it comes from, a native rewording) and **culturally off** details (formats, examples, payment or legal expectations). A text-only judge scores every answer against the source-locale intent.

```yaml
# glot.panel.yaml (optional)
product: Acme Invoices
routes:
  - route: /
    value_proposition: Create, send and track invoices with your team, and get paid faster.   # default: the source page's h1 — meta description
  - route: /pricing
personas:                       # optional; missing fields come from the defaults, a list = several personas
  de: { name: Ines, age: 58, occupation: Inhaberin eines Malerbetriebs, digital_literacy: low }
```

```sh
glot labs panel --base-url http://localhost:3000            # mock provider: deterministic, free (CI, demos)
glot labs panel --live --budget 1 [--model claude-sonnet-5-5] # the [llm] provider of glot.toml (vision)
glot labs panel --dry-run                                   # capture + estimated cost only
glot labs panel --reuse-capture --json                      # rerun the personas on .glot/panel/capture.json
glot labs panel --annotate                                  # also post the top problems to the live review ([server] + GLOT_TOKEN)
```

The estimated cost (vision tokens included, full output budget) is printed before any LLM call; a run whose estimate is above `--budget` (default `[llm] budget_usd`) does not start, and calls stop once it is reached. Output: **comprehension per locale** (5-second test + CTA understanding, 0–100) next to the source locale, the gap being the **localization debt**; the top problems ranked by how many personas and pages hit them, each with its key and a suggestion run through the glot validator (placeholders, plurals, tags, glossary — rejected ones are shown as such); `.glot/panel/report.html` (one self-contained accessible page with the screenshots and every verdict) and `report.json`.

Default personas cover en, fr, de, es, it, pt, nl, pl, ja, zh, ko, ar, ru, tr, hi, sv (others get a neutral one). They were reviewed against stereotypes: ordinary names, varied ages and jobs, and their "cultural expectations" are only observable localization conventions (number/date/currency formats, politeness register, legal pages, local payment methods), never temperaments attributed to a nationality. Personas are synthetic: treat their verdicts as leads to check with real users, not as user research.

## Cultural audit (`glot labs culture`)

A correct translation can still be wrong for a market. `glot labs culture` checks three things per target locale and gives each market a **readiness score** out of 100:

- **Code** (deterministic, with `file:line` and a fix): hard-coded `"en-US"` locales, US date formats and numeric dates, `$` glued to an amount, fixed `USD`, `toFixed` on prices, imperial units, US phone/ZIP patterns, a required "state" field, required first/last name fields, weeks starting on Sunday, plurals built with `+ "s"`, and string concatenation.
- **Catalog**: price/date arguments interpolated without an ICU `number`/`date` format, and pages a market expects: GDPR/privacy and VAT-inclusive prices in the EU, an Impressum in DE/AT, 特定商取引法 in JP, mentions légales in FR, LGPD in BR.
- **Copy** (LLM, per key and market): idioms, US references, symbols, humour, tone compared to `[variants]` and `style`, and legal wording. Each finding gives the problem, why it fails in that market, and a transcreated proposal.

Each rule costs at most 25 points, and US conventions do not count against `en-US`.

```sh
glot labs culture                          # every target locale, the [llm] provider of glot.toml (mock = free, deterministic)
glot labs culture --locale de,ja --keys pricing.title,pricing.cta
glot labs culture --no-llm --min-score 80  # code + catalog only; exit 1 when a market scores lower (CI)
glot labs culture --refresh                # ignore the cache in .glot/culture.json
glot labs culture --apply [--yes] [--json]
```

Proposals go through the glot validator and the number/price policy, so a rewrite that invents a figure is rejected. `--apply` writes the accepted ones as machine suggestions (`transcreation/<model>`, previous value kept) so they show up in `glot labs queue`; human or reviewed translations are never overwritten. Reports: `.glot/culture/report.{md,json}`.

## App Store and Google Play listings (`glot labs store`)

Store listings in fastlane's layout (`fastlane/metadata/<locale>/{name,subtitle,keywords,promotional_text,description,release_notes}.txt`, `fastlane/metadata/android/<locale>/{title,short_description,full_description,changelogs/}`), so `fastlane deliver` / `supply` upload them.

```sh
glot labs store init [--android]                       # draft en-US from Info.plist + UI strings; never overwrites
glot labs store generate [--locales fr,ja,es-MX] [--competitors Sherpa,VisaHQ] [--force] [--dry-run]
glot labs store notes [--since v1.2] [--version-code 42]   # CHANGELOG.md first section, else git log since the last tag
glot labs store check                                  # CI: exit 1 on any violation
```

- **Keywords are researched, not translated**: per country, the LLM lists what people type in that store (`ja`, `es-MX` and `pt-BR` get their own words), most relevant first, with a reason per word.
- **A deterministic validator owns the limits**: Apple name 30, subtitle 30, keywords 100 **bytes** (CJK is 3 bytes a character), promo 170, description 4000; Play title 30, short 80, full 4000, changelog 500. An overlong field is sent back once with its count; a field that still fails is not written.
- **Keyword field hygiene**: comma-separated without spaces, phrases split into words (Apple combines them), no word already in the name or subtitle, no competitor brand, no singular/plural pair, greedily packed to the byte. The report reads `ja  keywords 94/100 bytes` with dropped words and their reason.
- **Locale codes**: glot / `.lproj` / store spellings are mapped (`fr` → `fr-FR`, `es-MX` → Play `es-419`, `zh-Hans` → Play `zh-CN`, `ja` → Play `ja-JP`). Targets default to `glot.toml`, else the app's `.lproj` folders.
- **Release notes** keep what a user notices (chores, CI, docs, refactors and merges are filtered before the LLM sees them), one text per locale.

Existing target fields are kept unless `--force`. `--provider mock` runs offline.

## Which language next? (`glot labs markets`)

For each language you don't ship yet, glot combines four signals into a 0-100 score and one sentence:

- **Demand you already have**: share of visitors whose `Accept-Language` / `navigator.languages` asks for it, and their conversion rate, from [insights](self-hosting.md) (the existing anonymous counters, nothing new collected). Needs `[server]` in `glot.toml` and a token; without it, public data only. Observed demand weighs up to 60 % of the score from 500 visits.
- **Public market data**, embedded and dated (internet users by language, GDP per capita PPP, share of global e-commerce; sources in `crates/core/src/markets.rs`), discounted for markets that are hard to sell into (China, Russia, Iran).
- **Exact cost**: your whole catalog priced with the configured model (same estimate as `translate --dry-run`) plus review time at 800 words/hour.
- **Technical effort**: right-to-left layout, an extra webfont (CJK, Arabic, Thai, Devanagari), CLDR plurals with few/two/zero forms.

```sh
glot labs markets                 # table + "Add Portuguese (Brazil) (pt-BR): ~6 % of your visitors already ask for it, cost $0.42 (+ ~1.5 h of review), no technical effort → best opportunity"
glot labs markets --json --top 3
glot labs markets --add pt-BR     # appends it to target_locales in glot.toml, then runs translate --dry-run for it
```

The dashboard shows the same ranking as a **Next language** card on the Insights page, with one click to add the locale (`GET /api/projects/{slug}/markets`).

## Database content (`glot labs db`, `@glot-i18n/db`)

Product catalogs, help articles, pricing plans, notifications: text that lives in your database, not in JSON catalogs. glot translates it in the background into a shadow table `glot_translations(table, row_id, column, locale, value, source_hash, status, model, updated_at)` and your ORM reads it back with a source fallback. Postgres only.

```toml
# glot.toml
[[db.tables]]
table = "Product"                     # Postgres table (Prisma: model name unless @@map)
primary_key = "id"
context = "Catalog of an outdoor gear shop"
context_columns = ["category"]        # shown to the LLM, not translated
columns = { name = { context = "Product name", max_length = 60 }, description = "Product page description" }
```

```sh
glot labs db install --database-url $DATABASE_URL   # shadow table + trigger (marks stale on edit/delete, NOTIFY)
glot labs db sync [--locales de,fr] [--dry-run]     # missing + stale cells, idempotent, resumable, respects budget_usd
glot labs db sync --watch                           # resync on every change (LISTEN glot_db)
```

Each cell goes through the normal pipeline (validator, glossary, retries, fallback model) with its table, column, column note and the rest of the row as context. Results are written per chunk, so an interrupted sync resumes where it stopped; a cell the validator rejects is stored as `failed` and not retried until its source changes.

## A local translator in your project's voice (`glot labs distill`)

Every translation a human approved (`glot set`, `glot labs queue approve`, hand edits recorded in `glot.lock`) is a lesson in your terminology and tone. `glot labs distill` turns them into a translator that costs nothing per string: a local model (Ollama, LM Studio, llama.cpp server) that receives, with every request, the approved pairs closest to the string at hand plus your glossary and style.

```sh
glot labs distill                                   # dataset → .glot/distill/{train,eval}.jsonl
glot labs distill eval --model qwen3:8b             # local vs local+RAG vs your cloud model, on held-out pairs
glot labs distill translate --model qwen3:8b --escalate [--class label]
```

**Dataset.** Pairs that are reviewed or hand-written, still current (source and target unchanged since), valid, without secrets (API keys, tokens, private keys are skipped), deduplicated. Written as standard chat fine-tuning JSONL (`{"messages": [system, user, assistant]}`, the system message carrying style and the glossary terms of that string) with a stable hash split (`--eval-pct 10`). Fine-tune on it if you want; retrieval below needs no training.

**Retrieval (RAG).** Each string goes to the local model with its `k` nearest approved pairs (`-k 5`, character-bigram similarity over the reviewed pairs and the translation memory) and the matching glossary terms, through the same guarded pipeline as `glot translate`: validator, targeted retries, and with `--escalate` the cloud model of `glot.toml` only for what still fails. What validates is written like a translation-memory hit (provenance `tm`). `--provider openai-compatible --base-url http://localhost:1234/v1` for LM Studio or llama.cpp.

**Eval.** On the held-out split (never in the retrieval pool), three lanes without escalation: the local model alone, the local model with retrieval, your cloud model. Columns: judge score (`judge_model`, else the cloud model: it may favour its own family), similarity to your reviewed translation, validator pass rate (final and first try), cost, latency, and the same per key class (`label` ≤ 4 words, `sentence`, `icu`). It ends with a recommendation such as `qwen3:8b + RAG: $0.0000, 96 % of claude-haiku-5-5's judge score → route 'label' keys locally` and writes the matching routing (local first pass, cloud fallback via `llm.fallback_provider`, or `distill translate --class`) as a commented block at the end of `glot.toml` (`--no-write` to skip). `--cloud-provider mock` runs it offline.

## Beyond UI: emails, SVG and OG images (`glot labs assets`)

Emails, illustrations and social cards go through the same pipeline as the UI (keys, validator, glossary, lockfile, retries):

```sh
glot assets [--emails] [--svg] [--og] [--yes]   # i18nize -> translate -> preview -> check (idempotent)
glot assets i18nize | translate | preview | check
```

- **Emails** (`emails/`): React Email `.tsx` is converted like any JSX (keys `emails.<template>.*`, `export const subject` becomes a key); HTML/MJML templates are not rewritten, their copy (`<title>`/`<mj-title>` = subject, `<mj-preview>`/`.preheader`, `alt`) gets stable keys and `{{var}}` is checked as `{var}`. Previews per locale in `.glot/emails/<template>/<locale>.html` (`lang`/`dir` set, `PreviewProps` filled). Check: links identical to the source, subject ≤ 60 chars, preheader ≤ 100, `<Html>` wired with `dir` for RTL.
- **SVG** (`assets/*.svg`): `<text>`/`<tspan>`/`<title>` become `svg.<name>.*`. Each `<text>` gets a pixel budget (`data-max-width`, else the room left in the viewBox) in `.glot/layout.json`, so a translation that would overflow is rejected (`too_wide`) and retried *before* it is written; outputs `public/img/<locale>/<name>.{svg,png}` (resvg), and the rendered box is checked again.
- **OG images** (`og/*.tsx`, the `@vercel/og` flex-column subset): same keys/budgets (box width × `lineClamp`), rendered to `public/og/<locale>/<name>.png`.

Demo: `examples/assets-demo` (`./e2e.sh`, mock provider, fr/de/ja/ar).

```ts
import { glotPrisma } from "@glot-i18n/db";
const db = prisma.$extends(glotPrisma({ localized: { Product: ["name", "description"] } }));
await db.withLocale("de").product.findMany(); // translated, source value when missing or stale

import { glotDrizzle } from "@glot-i18n/db";        // node-postgres or postgres.js
await glotDrizzle(db, { localized: { products: ["name"] } }).withLocale("de").localize("products", rows);
```

A translation is served only if its `source_hash` matches the current source (sha256 prefix), so an edit falls back to the source immediately, even before the trigger or the next sync. Prisma Migrate users can ship `packages/db/migrations/0001_glot_translations.sql` and still run `glot labs db install` once for the per-table triggers. Full offline demo (30 products, mock LLM): `examples/db-demo` (`pnpm e2e`).
