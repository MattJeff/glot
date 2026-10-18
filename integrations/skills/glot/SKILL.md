---
name: glot
description: Internationalize, translate and validate a web app's UI strings with glot (MIT CLI + MCP server). Use when the user wants the app in other languages (i18n, l10n, localization, multilingual, translations), adds or changes user-facing text in a project with glot.toml, next-intl, react-i18next or locale JSON catalogs, or asks for AI-translation provenance (EU AI Act art. 50).
license: MIT
compatibility: Needs Node.js (npx) or the glot binary on PATH. Translating needs ANTHROPIC_API_KEY or OPENAI_API_KEY (llm.provider = "mock" works offline).
metadata:
  homepage: https://github.com/MattJeff/glot
  version: "1"
---

# glot: localize the app in this repo

glot moves hardcoded UI strings into catalogs, translates them with an LLM, and validates every translation deterministically (placeholders, ICU plurals, glossary, length). Catalogs stay plain JSON in git; `glot.lock` records what was translated, by which model, and what a human reviewed.

Run it as `glot` when on PATH, otherwise `npx -y glot-i18n` (same arguments). `scripts/glot.sh` picks the right one.

## One step

`glot localize --yes` (add `--targets fr,de,es` on a new project): init if needed → i18nize → translate → check → status. Idempotent: rerun it after any UI change. `glot localize --dry-run` shows the plan and cost without writing.

Ask the user which languages when they did not say. If no API key is set, stop and name the variable glot.toml expects (or use `--provider mock` to test offline).

## Step by step (when you need control)

1. No `glot.toml`: `glot init --yes --targets fr,de,es`.
2. `glot extract --json` lists hardcoded strings, missing and unused keys.
3. `glot i18nize --dry-run`, review, then `glot i18nize --yes`: rewrites components to `t("key")`, fills the source catalog, wires next-intl / react-i18next.
4. `glot translate --dry-run` (plan + cost), then `glot translate --yes`.
5. `glot check` must exit 0. Fix errors with `glot set <locale> <key> "<value>"` or `glot translate --key <key> --force`.
6. Running app: `glot fit --base-url http://localhost:3000 --routes /,/login` shortens overflowing translations.

## Adding or changing UI text later

- With the glot MCP server (`glot mcp`): `propose_string` first (dedupes, glossary), then `add_string`, use the key in code, then `translate` and `check`.
- Without MCP: `glot add <key> "<text>" --note "<context>"`, then `glot translate --yes`, then `glot check`.
- Brand/product terms: `glot glossary add <Term> --dnt` or `--fr "<translation>"`.

## Rules

- User-facing text lives only in the source catalog: never hardcoded in components, never written by hand in target catalogs.
- Keep placeholders (`{name}`, `{{count}}`) and ICU plurals; never build sentences by concatenation.
- Commit `glot.lock` with the catalogs.

## More

- Full command list and flags: [references/commands.md](references/commands.md).
- AI-translation disclosure (provenance export, runtime badge): [references/provenance.md](references/provenance.md).
- Prompt for app builders (Lovable, Bolt, v0) or another agent: `glot agent-prompt`.
