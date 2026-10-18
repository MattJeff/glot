# glot

**AI-native localization for SaaS.** Git is the source of truth, your coding agent is the interface, and every LLM output goes through deterministic code before it touches your catalogs.

`glot` is a Rust CLI + MCP server + optional API/dashboard. It works with the i18n library you already use (next-intl, i18next, FormatJS…) and ships no runtime.

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

## Quickstart

```sh
cargo install --git https://github.com/MattJeff/glot glot-cli   # or grab a binary from Releases

glot init                          # detects your stack, writes glot.toml + agent rules
claude mcp add glot -- glot mcp    # give Claude Code live i18n context
export ANTHROPIC_API_KEY=...
glot translate                     # translates missing/stale strings, validated
glot check                         # CI-friendly: --format text|json|sarif|github
```

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

Tests: `cargo test --workspace` (needs `DATABASE_URL`), `cd web && pnpm lint && pnpm build`, `cd packages/visual-qa && pnpm test`.

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
