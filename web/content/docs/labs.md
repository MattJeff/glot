# Labs

Labs are the parts of glot outside the core loop (`init` → `i18nize` → `translate` → `check`, with pixel budgets and the MCP server). They work and are tested, but they are younger, less used, and their flags may change between minor versions. In the CLI they live under `glot labs <command>`; the old spelling (`glot seo`, `glot db`…) still works.

| | What it does | Where |
|---|---|---|
| Plural coverage | Every plural/select branch per locale, measured against the pixel budget, optionally rendered | `glot labs coverage` |
| Review | Back-translation with explanations; `--stale` sorts cosmetic from real source changes | `glot labs review` |
| Register variants | Formal/informal catalogs generated from the base locale | `[variants]` in `glot.toml` |
| Task parity | An agent runs the same tasks in every locale and compares success | `glot labs parity` |
| SEO | hreflang/canonical/metadata audit per locale, localized keywords | `glot labs seo` |
| Database content | Translates rows into a shadow table, read back by your ORM | `glot labs db`, `@glot/db` |
| Emails, SVG, OG images | Same pipeline as the UI, rendered per locale | `glot labs assets` |
| Unused keys | Source keys never referenced in code (`--prune`) | `glot labs unused` |
| Insights, experiments, feedback, on-demand translation | Growth features of the [self-hosted server](self-hosting.md) | dashboard, **Labs** section |

## Plural coverage (`glot labs coverage`)

A page shows `5 plików`; nobody ever looks at `3 pliki` or `22 pliki`. `glot coverage` enumerates every branch of every plural / select / selectordinal message (ICU and MF2), per locale: each CLDR category of the locale with a real witness number (`pl`: 1, 3, 11, 1.5 · `ar`: 0, 1, 2, 3, 11, 100), each explicit `=0`, each select key, and their combinations when nested. Each branch is rendered as text and measured against the key's pixel budget (`.glot/layout.json` from `glot measure`, no browser).

```sh
glot coverage                                   # table: locale × category, branches, seen in a browser, failing, too wide
glot coverage --render --base-url http://localhost:3000 --routes /   # mount every branch on the app page
glot coverage --locale pl,ar --json
```

`--render` builds a test page per locale (one box per branch with the measured width, lines and font) and mounts it on your app's page so its CSS and webfonts apply: no instrumentation. visual-qa (`--harness`) reports overflow and truncation per branch; results are kept in `.glot/coverage.json` per branch text, so an edited translation counts as unseen again. The report reads like "pl few: 3 of 8 seen in a browser (37 %), 1 failing". Exit 1 on any failing or too-wide branch.

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

## Database content (`glot labs db`, `@glot/db`)

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
import { glotPrisma } from "@glot/db";
const db = prisma.$extends(glotPrisma({ localized: { Product: ["name", "description"] } }));
await db.withLocale("de").product.findMany(); // translated, source value when missing or stale

import { glotDrizzle } from "@glot/db";        // node-postgres or postgres.js
await glotDrizzle(db, { localized: { products: ["name"] } }).withLocale("de").localize("products", rows);
```

A translation is served only if its `source_hash` matches the current source (sha256 prefix), so an edit falls back to the source immediately, even before the trigger or the next sync. Prisma Migrate users can ship `packages/db/migrations/0001_glot_translations.sql` and still run `glot labs db install` once for the per-table triggers. Full offline demo (30 products, mock LLM): `examples/db-demo` (`pnpm e2e`).
