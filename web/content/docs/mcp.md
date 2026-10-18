# MCP server

`glot mcp` runs an [MCP](https://modelcontextprotocol.io) server on stdio over the project in the current directory. It gives your coding agent your keys, where they are used, the glossary and the notes, so it adds and translates strings while you code instead of hardcoding them.

```sh
claude mcp add glot -- npx -y glot-i18n mcp      # Claude Code
codex mcp add glot -- npx -y glot-i18n mcp       # Codex
```

Cursor, Windsurf and other clients: `{"mcpServers":{"glot":{"command":"npx","args":["-y","glot-i18n","mcp"]}}}`. With `glot` on your PATH, use `glot mcp` directly. It is listed in the official MCP registry as `io.github.MattJeff/glot`.

The server starts in any repo; until a `glot.toml` exists, every tool answers with how to run `glot init`.

## Workflow the server teaches

The server instructions tell the agent: before adding UI text, call `propose_string` (and reuse duplicates), then `add_string`, replace the literal with the key in code, `translate`, `check`. Use `get_context` to understand a file's strings. Never edit catalog JSON by hand for new keys.

## Tools

| Tool | What it does |
|---|---|
| `status` | Number of keys, per-locale counts (translated, missing, stale, reviewed, errors, warnings) and the pending plan. Call first. |
| `get_context` | For a file: keys it uses, keys missing from the source catalog, hardcoded strings. For each key: usages (file:line), UI role, component, note, siblings, value in every locale, open issues, applicable glossary terms. |
| `propose_string` | Dry run before adding a string: normalized key, existing keys with the same or near-identical text (reuse them), glossary terms, syntax problems. Creates nothing. |
| `add_string` | Adds a source string with an optional note. Validates syntax; refuses to change an existing key's text unless `overwrite: true`. |
| `translate` | Translates the delta with validation and the glossary. Returns a compact summary (counts, rejected keys, cost), never whole catalogs. `dry_run: true` for plan + estimate. |
| `check` | Deterministic QA, no network. At most 100 issues per call with the total; page with `offset`. |
| `set_translation` | Sets one translation by hand (recorded as human, never overwritten by `translate`). Nothing is written if it fails validation. |
| `find_hardcoded` | User-visible literals not wrapped in a translation call, with file:line, UI role, component and a suggested key. |
| `i18nize_file` | Converts a whole file in one step (same engine as `glot i18nize`). `dry_run` (default `true`) returns the diff, the keys and the skipped strings; `dry_run: false` writes the file and the source catalog. |
| `glossary_lookup` | Glossary terms; with `text`, only the terms it contains. |
| `glossary_upsert` | Adds or updates a term. If it conflicts with an existing translation, the human is asked (elicitation) when the client supports it; otherwise nothing is written unless `overwrite: true`. |
| `remove_keys` | Deletes keys from every catalog and the lock; warns if they are still referenced in code. |
| `review_translation` | Back-translation into the source language, explanation and verdict (`same`, `nuance`, `different`) with a suggestion. Cached in `.glot/review.json`. |
| `mine_glossary` | Finds glossary terms and inconsistent translations; `apply: true` writes the glossary and retranslates those keys. |

## Resources

| URI | Content |
|---|---|
| `glot://config` | Project configuration (`glot.toml`) |
| `glot://glossary` | Glossary terms |
| `glot://status` | Per-locale status and pending plan |
| `glot://catalog/{locale}` | Flat `key → value` catalog of one locale (template) |

## Prompts

| Prompt | Argument | Steps it gives the agent |
|---|---|---|
| `internationalize_file` | `path` | `get_context` → `find_hardcoded` → `propose_string` (reuse duplicates, ICU arguments instead of concatenation) → `add_string` with notes → edit the file → `translate` + `check` |
| `review_translations` | `locale` | `status` + `check` → `get_context` on each issue → fix with `set_translation` → `glossary_upsert` for inconsistent terms → `check` again and summarize |

In Claude Code they appear as `/mcp__glot__internationalize_file` and `/mcp__glot__review_translations`.
