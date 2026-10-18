# glot

**AI-native localization for SaaS.** Git is the source of truth, your coding agent is the interface, and every LLM output goes through deterministic code before it touches your catalogs.

`glot` is a Rust CLI + MCP server + optional API/dashboard. It works with the i18n library you already use (next-intl, i18next, FormatJS…) and ships no runtime. MIT, bring your own LLM key.

![glot extract → i18nize → extract on the demo app: 117 hardcoded strings, then none](web/public/launch/i18nize.gif)

<sub>Real run on [`examples/demo-saas`](examples/demo-saas) with the mock provider (key names come from heuristics; a real model names them better). Recorded by Playwright, see [`docs/launch`](docs/launch).</sub>

```sh
npx glot-i18n init && npx glot-i18n i18nize && npx glot-i18n translate
```

**Docs**: [Install & quickstart](web/content/docs/getting-started.md) · [Concepts](web/content/docs/concepts.md) · [CLI reference](web/content/docs/cli.md) · [MCP server](web/content/docs/mcp.md) · [Integrations](web/content/docs/integrations.md) · [Frameworks](web/content/docs/frameworks.md) · [FAQ & costs](web/content/docs/faq.md) (rendered with search at `/docs` in the dashboard)

| Alt+click any text to edit it in the running app (`glot dev`) | Every locale side by side (dashboard) |
|---|---|
| ![overlay](web/public/launch/overlay.gif) | ![locale matrix](web/public/launch/locale-matrix.gif) |

## Why glot (vs Lokalise, Crowdin, Lingo.dev)

| | glot |
|---|---|
| **Deterministic validator** | Every translation is checked for ICU / i18next / MessageFormat 2 syntax, placeholders, CLDR plural categories, glossary terms and length. Failures are retried with the issues in the prompt, then escalated to a fallback model. Nothing invalid is written. |
| **Lockfile** | `glot.lock` hashes source and target per key. Only the delta is translated, and a translation edited by hand is **never** overwritten without `--force`. |
| **Glossary as code** | `glossary.toml` lives in your repo, is reviewed in PRs, and is enforced by the validator (required and forbidden translations, do-not-translate terms). |
| **Living context via MCP** | `glot mcp` gives Claude Code (or any MCP agent) your keys, usages in code (component, UI role, file:line), glossary and notes, so new strings are added and translated while you code. |
| **Visual QA** | `glot-visual-qa` screenshots every route × locale × viewport and flags overflowing, truncated, overlapping, untranslated or tofu text, with optional shorter-text suggestions. |
| **BYO key** | Anthropic, OpenAI, any OpenAI-compatible endpoint (Ollama…). Cost is estimated up front and budgeted. |
| **Open source** | MIT. No per-seat or per-word pricing, no lock-in: your catalogs stay plain JSON in Git. |

## Install

```sh
npx glot-i18n init                                                              # no install: npm fetches the binary
npm i -g glot-i18n                                                              # `glot` on PATH (macOS, Linux, Windows)
brew install mattjeff/glot/glot                                                 # Homebrew (macOS, Linux)
curl -fsSL https://raw.githubusercontent.com/MattJeff/glot/main/integrations/install.sh | sh   # ~/.local/bin
cargo install --git https://github.com/MattJeff/glot glot-cli                   # from source
```

Every prebuilt binary comes from the [GitHub release](https://github.com/MattJeff/glot/releases) and is checked against its SHA-256 before install (pinned in the npm package and the formula; release digests for `install.sh`).

## Quickstart

```sh
glot init                          # detects your stack, writes glot.toml + agent rules
claude mcp add glot -- glot mcp    # give Claude Code live i18n context
export ANTHROPIC_API_KEY=...
glot translate                     # translates missing/stale strings, validated
glot check                         # CI-friendly: --format text|json|sarif|github
```

### LLM providers

| Variable | Use |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic key (default provider; another name via `llm.api_key_env`) |
| `ANTHROPIC_WORKSPACE_ID` | Workspace to bill, required with organization-level keys (`sk-ant-usr-…`). On the server, the per-project setting wins. |
| `OPENAI_API_KEY` | `provider = "openai"` / `"openai-compatible"` |

The system prompt (rules + glossary) is a cached prefix, so repeated batches pay ~10 % of its price. `glot translate --batch` sends the first pass through Anthropic's Message Batches API (-50 %, asynchronous, resumable).

Other commands: `glot status`, `glot extract` (hardcoded strings, missing/unused keys), `glot set <locale> <key> "<value>"`, `glot glossary`, `glot push|pull`.

### GitHub Action

```yaml
on: pull_request
permissions: { contents: write, pull-requests: write }
jobs:
  i18n:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: ${{ github.event.pull_request.head.repo.full_name }}
          ref: ${{ github.head_ref }}
      - uses: MattJeff/glot/action@v0
        with:
          # Fork PRs get no secrets: only check them, translate on same-repo branches.
          command: ${{ github.event.pull_request.head.repo.full_name == github.repository && 'both' || 'check' }}
          anthropic-api-key: ${{ secrets.ANTHROPIC_API_KEY }}
```

It validates catalogs (inline annotations), translates the delta, commits catalogs + `glot.lock` to the PR branch and keeps one PR comment up to date with coverage, issues and cost. See [`action/action.yml`](action/action.yml).

### Visual QA

```sh
cd packages/visual-qa && pnpm install && pnpm exec playwright install chromium
node src/cli.ts --base-url http://localhost:3100 --routes /,/login --locales fr,de,ja,ar --suggest
```

Writes `.glot/visual-qa/report.{json,md}` and screenshots; exits 1 on overflow (`--fail-on`). `report.md` is ready to post as a PR comment. Add `data-glot-key="auth.login.cta"` to elements to get ready-to-run `glot set` fixes.

### Plural coverage (`glot coverage`)

A page shows `5 plików`; nobody ever looks at `3 pliki` or `22 pliki`. `glot coverage` enumerates every branch of every plural / select / selectordinal message (ICU and MF2), per locale: each CLDR category of the locale with a real witness number (`pl`: 1, 3, 11, 1.5 · `ar`: 0, 1, 2, 3, 11, 100), each explicit `=0`, each select key, and their combinations when nested. Each branch is rendered as text and measured against the key's pixel budget (`.glot/layout.json` from `glot measure`, no browser).

```sh
glot coverage                                   # table: locale × category, branches, seen in a browser, failing, too wide
glot coverage --render --base-url http://localhost:3000 --routes /   # mount every branch on the app page
glot coverage --locale pl,ar --json
```

`--render` builds a test page per locale (one box per branch with the measured width, lines and font) and mounts it on your app's page so its CSS and webfonts apply: no instrumentation. visual-qa (`--harness`) reports overflow and truncation per branch; results are kept in `.glot/coverage.json` per branch text, so an edited translation counts as unseen again. The report reads like "pl few: 3 of 8 seen in a browser (37 %), 1 failing". Exit 1 on any failing or too-wide branch.

### Register variants (`[variants]`)

Formal vs informal is not a separate language, but it is a separate catalog. Declare variants per base locale:

```toml
[variants]
fr = ["formal", "informal"]   # vous / tu
de = ["sie", "du"]
en = ["plain"]                # plain language (accessibility)
ja = ["keigo"]
```

They are stored as BCP 47 private-use locales, `messages/fr-x-informal.json`. `glot translate` generates them **from the base locale** (`fr`, not `en`) after the base, with the register in the prompt, so both share terminology; they are tracked in `glot.lock` against the base text (a base change makes the variant stale) and `glot check` validates them against the base (same placeholders, plural categories, tags). At runtime it's just another locale: pick it from a user preference, e.g. with next-intl set the locale cookie (`NEXT_LOCALE=fr-x-informal`) when the user chooses "tutoiement", and add the variant tags to your `locales` list.

### Task parity (`glot parity`)

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

### Database content (`glot db`, `@glot/db`)

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
glot db install --database-url $DATABASE_URL   # shadow table + trigger (marks stale on edit/delete, NOTIFY)
glot db sync [--locales de,fr] [--dry-run]     # missing + stale cells, idempotent, resumable, respects budget_usd
glot db sync --watch                           # resync on every change (LISTEN glot_db)
```

Each cell goes through the normal pipeline (validator, glossary, retries, fallback model) with its table, column, column note and the rest of the row as context. Results are written per chunk, so an interrupted sync resumes where it stopped; a cell the validator rejects is stored as `failed` and not retried until its source changes.

### Beyond UI: emails, SVG and OG images (`glot assets`)

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

A translation is served only if its `source_hash` matches the current source (sha256 prefix), so an edit falls back to the source immediately, even before the trigger or the next sync. Prisma Migrate users can ship `packages/db/migrations/0001_glot_translations.sql` and still run `glot db install` once for the per-table triggers. Full offline demo (30 products, mock LLM): `examples/db-demo` (`pnpm e2e`).

## Use with AI agents

- **Claude Code**: `claude plugin marketplace add MattJeff/glot && claude plugin install glot@glot` adds a `glot` skill (Claude uses it whenever you ask for a multilingual app), `/glot:i18nize`, `/glot:translate` and the MCP server. MCP only: `claude mcp add glot -- npx -y glot-i18n mcp`.
- **Cursor / Windsurf / Codex**: drop-in rules in [`integrations/agents`](integrations/agents) (`.mdc`, Windsurf rule, `AGENTS.md`); MCP command `npx -y glot-i18n mcp` (stdio, run inside a project with `glot.toml`). Listed in the official MCP registry as `io.github.MattJeff/glot`.
- **Lovable, Bolt, v0, Replit**: paste the [knowledge snippet](integrations/ai-builders-prompt.md) so generated code is i18n-ready, then [Translate this app](integrations/ai-builders.md#translate-this-app): one workflow run does `init` + `i18nize` + `translate` and opens a PR.

[![Translate this app with glot](https://img.shields.io/badge/Translate_this_app-glot-2563eb?logo=googletranslate&logoColor=white)](integrations/ai-builders.md#translate-this-app)

Show your coverage with a [`translated by glot · N languages · X%` badge](integrations/README.md#badge). All integrations: [`integrations/`](integrations).

## Demo

[`examples/demo-saas`](examples/demo-saas) is "Acme Invoices", a small Next.js SaaS with every string hardcoded in English (plurals by concatenation, copy in config objects, fixed-width buttons, ambiguous "Save"/"Archive" labels). [`examples/demo.sh`](examples/demo.sh) takes a throwaway copy from hardcoded English to validated catalogs, with commentary at each step:

```sh
examples/demo.sh                                       # mock LLM, no key needed
DEMO_PAUSE=1 examples/demo.sh                          # pause between steps for a live demo
GLOT_REAL=1 ANTHROPIC_API_KEY=... examples/demo.sh     # real translations
KEEP=1 examples/demo.sh                                # keep the copy, then `pnpm install && pnpm dev` (port 3200)
```

Steps: `init` → `extract` (117 hardcoded strings) → `i18nize` (24 files rewritten to next-intl, 132 keys) → minimal next-intl wiring + `next dev` → `translate --visual` → `check` → `fit` (German overflow fixed in the running app) → `review` → `glossary mine`. `NO_APP=1` skips the dev server (and thus `--visual`/`fit`). Commands missing from your build are skipped with a notice.

## Architecture

Rust workspace (`crates/core`, `extract`, `llm`, `mcp`, `cli`, `server`), Next.js dashboard in `web/`, Playwright QA in `packages/visual-qa`, GitHub Action in `action/`. Details, file formats and the HTTP API: [ARCHITECTURE.md](ARCHITECTURE.md).

## Local development

```sh
docker compose up -d postgres      # postgres://glot:glot@localhost:5433/glot
cargo run -p glot-server           # API on :8790
cd web && pnpm install && pnpm dev # dashboard on :3100
```

Tests: `cargo test --workspace` (needs `DATABASE_URL`), `cd web && pnpm lint && pnpm build`, `cd packages/visual-qa && pnpm test`, `cd packages/agent-qa && pnpm test`.

### Docker (full stack)

```sh
GLOT_SECRET_KEY=$(openssl rand -hex 32) docker compose --profile full up -d --build --wait
curl localhost:8790/healthz                # ok (migrations run at startup)
open http://localhost:3100                 # dashboard; /api and /cdn are proxied to the server
docker compose --profile full down         # add -v to drop the database volume
```

Host ports default to 5433 / 8790 / 3100; override with `GLOT_PG_PORT`, `GLOT_API_PORT`, `GLOT_WEB_PORT` (set `GLOT_WEB_ORIGIN` to the web URL if you change `GLOT_WEB_PORT`). The web image bakes the API URL into the Next build (rewrites are resolved at build time), so changing `GLOT_API_URL` means rebuilding `web`. Cold build: ~7 min server (mostly the Rust image pull, ~1.5 min compile), ~2 min web; images ~170 MB (server) and ~440 MB (web).

## Dogfooding

The dashboard in `web/` is translated by glot itself. Any PR touching `web/messages/en.json` runs [`dogfood.yml`](.github/workflows/dogfood.yml), which builds the CLI from the PR and runs our own action in `web/` to translate, validate, commit and comment.

## License

[MIT](LICENSE) © 2026 glot contributors
