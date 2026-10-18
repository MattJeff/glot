# Concepts

glot's rule: the LLM is always surrounded by deterministic code that can say no. Git is the source of truth, your catalogs stay plain JSON, and glot ships no runtime of its own (except the optional [runtime fallback](runtime.md)).

## Validator

Every translation, whether it comes from the LLM, a human (`glot set`, the dashboard, the overlay) or an import, is checked by `glot_core::validate` against its source string. It knows three message syntaxes (`syntax = "icu" | "i18next" | "mf2"`) and reports issues with a code and a severity:

| Code | Severity | Catches |
|---|---|---|
| `syntax_error` | error | Unbalanced braces, broken ICU/MF2 |
| `placeholder_missing` / `placeholder_extra` | error | `{name}` dropped or invented |
| `plural_category_missing` / `plural_category_extra` | error | Plural forms that do not match the target locale's CLDR categories (`one/few/many/other` in Polish, only `other` in Japanese) |
| `keyword_translated` | error | `plural`, `select`, `other`… translated by mistake |
| `tag_mismatch` | error | Rich-text tags (`<b>`, `<link>`) lost or renamed |
| `glossary_violation` / `do_not_translate_violation` | error | A required term missing, a forbidden variant used, a brand name translated |
| `empty`, `untranslated` | error / warning | Empty output, or identical to the source |
| `too_long` | warning | Longer than `max_length_ratio × source` (default 1.6) |
| `too_wide` | warning | Renders wider than its measured box (see [pixel budgets](#pixel-budgets)) |
| `whitespace_mismatch` | warning | Leading/trailing spaces changed |
| `low_score` | warning | Judge score below `qa.score_threshold` (optional judge model) |

During `glot translate`, a failed output is retried with the issues in the prompt, then escalated to `fallback_model`. If it still fails, nothing is written and the key is reported as rejected. `glot check` runs the same checks over every catalog with no network and exits 1 on errors (`--format text|json|sarif|github`).

## Lockfile

`glot.lock` is a sorted, generated JSON file you commit. For each locale and key it records a hash of the source and of the target, where the target came from and its score:

```json
{ "version": 1,
  "notes": { "auth.login.cta": "Primary button of the login page" },
  "entries": { "fr": { "auth.login.cta": {
      "source_hash": "9f86d081884c7d65", "target_hash": "2c26b46b68ffc68f",
      "origin": { "kind": "machine", "model": "claude-haiku-5-5" },
      "score": 92, "reviewed": false } } } }
```

What it buys you:

- **Only the delta is translated.** A key is translated when it is missing, or *stale* (its source hash changed). Everything else is skipped, so a re-run costs nothing.
- **Human edits are safe.** If the target in the catalog no longer matches `target_hash`, someone edited it by hand. `translate` never overwrites it unless you pass `--force`.
- **Notes for translators.** `notes` holds per-key context that reaches the prompt.
- **Reviewable in PRs.** Catalog and lock changes show up together in the diff.

`glot review --stale` sorts stale keys into cosmetic source edits (keep the translation, refresh the hash) and real changes (retranslate).

## Glossary

`glossary.toml` lives in your repo and is reviewed like code:

```toml
[[term]]
term = "Workspace"
description = "A team's shared space"
translations = { fr = "Espace de travail", de = "Arbeitsbereich" }
forbidden = { fr = ["Espace de travaux"] }

[[term]]
term = "glot"
do_not_translate = true
```

The terms that occur in a batch go into the system prompt, and the validator enforces them on the output. Manage it with `glot glossary list|add`, or let glot find terms: `glot glossary mine` looks for frequent product terms translated inconsistently across the catalog and, with `--apply`, writes the glossary and retranslates the affected keys.

## Pixel budgets

Length ratios are a rough proxy. glot can measure the real space a string has:

```sh
glot measure --base-url http://localhost:3000 --routes /,/pricing
```

`measure` renders the source locale, maps each element to its key (from the rendered text, no instrumentation) and stores its box in `.glot/layout.json` (commit it): max width in pixels, max lines, font family, size, weight, letter spacing and transform. Webfonts are converted to TTF in `.glot/fonts/`.

From then on the validator shapes each translation with rustybuzz, without a browser, and raises `too_wide` when it does not fit. The prompt carries the budget ("must fit in 110px at Inter 14px/500 ≈ 16 Latin chars") so the retry loop fixes it before anything is written.

For what is already written, `glot fit` captures each locale in the running app, shortens the strings that overflow, re-renders and repeats (`--max-iterations`, default 3). With a production build, pass `--build-cmd`.

## Context

The LLM gets more than the string. For each key, glot assembles a context from the code and the repo:

- **where it is used**: `file:line` of every reference, the component, and the UI role (button, heading, label, placeholder, aria, title, link, toast, text);
- **neighbors**: sibling keys with the same prefix and their source text, so "Save" next to "Cancel" reads as a verb;
- **notes**: from `glot.lock` notes, `glot add --note`, or the dashboard;
- **the glossary terms** that occur in it, and its **pixel budget** if measured;
- optionally a **screenshot crop** of where it renders: `glot translate --visual --base-url … --routes …`.

The same context is exposed to coding agents through the MCP tool `get_context` (see [MCP](mcp.md)), so new strings are named and placed consistently while you code.

## Costs and budgets

You bring your own key (Anthropic, OpenAI, any OpenAI-compatible endpoint such as Ollama). `glot translate --dry-run` prints an estimate before any call, and `llm.budget_usd` caps a run. Details in the [FAQ](faq.md).
