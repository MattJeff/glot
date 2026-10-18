---
name: glot
description: Internationalize and translate a web app with glot. Use whenever the user wants their app in other languages, multilingual/i18n/l10n/localization/translations, or when adding or changing user-facing UI text in a project that has glot.toml, next-intl, react-i18next or locale JSON catalogs.
---

# glot: i18n for the app in this repo

glot moves hardcoded UI strings into catalogs, translates them with an LLM, and validates every translation deterministically (placeholders, ICU plurals, glossary, length). Catalogs stay plain JSON in git.

Run it as `glot` if it is on PATH, otherwise `npx -y glot-i18n` (same arguments).

## When the project has no glot.toml yet

1. Make sure an i18n library is installed. Next.js App Router: `next-intl`. Vite/CRA/React: `react-i18next` + `i18next`. Install it with the project's package manager if absent.
2. `glot init --yes --targets fr,de,es` (ask the user which languages if they did not say). This writes glot.toml, glossary.toml, .mcp.json, agent rules and a GitHub workflow.
3. Continue with the workflow below.

## Workflow: i18nize → translate → check → fit

1. `glot extract --json`: list hardcoded strings, missing and unused keys.
2. `glot i18nize --dry-run`, review, then `glot i18nize --yes`: rewrites components to `t("key")` and fills the source catalog. Then wire the library once if it is not already (next-intl: `i18n/request.ts` + `NextIntlClientProvider`; react-i18next: an `i18n.ts` init imported by the entry file, loading the catalogs from glot.toml).
3. `glot translate --dry-run` shows the plan and cost; `glot translate --yes` translates missing/stale keys. Needs `ANTHROPIC_API_KEY` (or `OPENAI_API_KEY`, per glot.toml). Never hand-write target-locale catalogs.
4. `glot check` must pass (exit 0) before you finish. Fix what it reports with `glot set <locale> <key> "<value>"` or by retranslating with `glot translate --key <key> --force`.
5. If the app runs locally: `glot fit --base-url http://localhost:3000 --routes /,/login` shortens translations that overflow the layout.

## Adding or changing UI text later

- With the glot MCP server: call `propose_string` first (it finds duplicates to reuse and glossary terms), then `add_string`, use the key in code, then `translate` and `check`. `get_context` shows a file's keys, usages and hardcoded strings.
- Without MCP: `glot add <key> "<text>" --note "<where/what it is>"`, then `glot translate --yes`, then `glot check`.
- Brand and product terms go in the glossary: `glot glossary add <Term> --dnt` or `--fr "<translation>"`.

## Rules

- Write user-facing text only in the source catalog, never hardcoded in components, never in target catalogs.
- Keep placeholders (`{name}`, `{{count}}`) and ICU plural forms; never build sentences by concatenation.
- `glot.lock` is part of the change: commit it with the catalogs.
