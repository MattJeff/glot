# Install & quickstart

glot is a single Rust binary: a CLI, an MCP server for coding agents and, optionally, an API + dashboard. It works with the i18n library you already use (next-intl, i18next/react-i18next, FormatJS) and keeps your catalogs as plain JSON in Git.

## Install

Pick one channel. They all install the same prebuilt binary from the [GitHub release](https://github.com/MattJeff/glot/releases), checked against its SHA-256.

| Channel | Command |
|---|---|
| npm, no install | `npx glot-i18n init` |
| npm, global | `npm i -g glot-i18n` |
| Homebrew (macOS, Linux) | `brew install mattjeff/glot/glot` |
| Shell script (`~/.local/bin`) | `curl -fsSL https://raw.githubusercontent.com/MattJeff/glot/main/integrations/install.sh \| sh` |
| From source | `cargo install --git https://github.com/MattJeff/glot glot-cli` |

Check it: `glot --version`.

Coding agents get it through their own channels: the Claude Code plugin (`claude plugin marketplace add MattJeff/glot`), the MCP registry entry `io.github.MattJeff/glot`, or any MCP client with `npx -y glot-i18n mcp`. See [Integrations](integrations.md).

## 1. `glot init`

Run it at the root of your app:

```sh
glot init                       # interactive
glot init --yes --targets fr,de,ja --provider anthropic
```

It detects your stack and catalogs, then writes:

- `glot.toml`: source and target locales, catalog paths, format, message syntax, LLM settings;
- `glossary.toml`: your product terms (empty to start);
- agent rules: `CLAUDE.md`, `AGENTS.md`, `.claude/skills/glot`, `.cursor/rules/glot.mdc`, `.mcp.json`;
- a GitHub workflow (`.github/workflows/glot.yml`) that checks and translates on pull requests.

A minimal `glot.toml`:

```toml
source_locale = "en"
target_locales = ["fr", "de", "ja"]
files = ["messages/{locale}.json"]
format = "auto"          # auto | json-nested | json-flat
syntax = "icu"           # icu | i18next | mf2

[llm]
provider = "anthropic"   # anthropic | openai | openai-compatible | mock
model = "claude-haiku-5-5"
fallback_model = "claude-sonnet-5-5"
api_key_env = "ANTHROPIC_API_KEY"
budget_usd = 5.0
```

## 2. `glot i18nize` (apps with hardcoded strings)

If your UI text is still hardcoded, `glot i18nize` rewrites it to translation calls:

```sh
glot extract                    # list hardcoded strings, missing and unused keys
glot i18nize --dry-run          # show the diff and the keys it would create
glot i18nize --yes              # rewrite the code, fill the source catalog, wire the library
```

It handles JSX text with expressions and inline tags, attributes, concatenations and template literals, and turns count-based concatenations into ICU plurals. Key names come from one LLM call; the rewrite itself is deterministic. It also wires the runtime it detects (next-intl provider, config and proxy; react-i18next init; see [Frameworks](frameworks.md)). Pass `--no-setup` to only rewrite code, `--switcher` to also get a `<LocaleSwitcher />`.

On the bundled demo app (`examples/demo-saas`, a small Next.js invoicing SaaS), `extract` finds 117 hardcoded strings. After `i18nize`, `extract` reports nothing left. A few strings are deliberately skipped and listed with the reason (for example sample data in module-level arrays).

## 3. `glot translate`

### Try it without a key

The `mock` provider needs no API key and no network: it writes placeholder translations (`⟦fr⟧ …`) through the same pipeline (plan, validator, lockfile, catalogs), so you can see what glot does before paying for anything.

```sh
glot init --yes --targets fr,de --provider mock
glot translate --yes
glot status
```

Switch to a real provider later by editing `[llm]` in `glot.toml`, then run `glot translate --force` to replace the placeholders.

### With an LLM key

```sh
export ANTHROPIC_API_KEY=...
glot translate --dry-run        # plan + estimated cost, writes nothing
glot translate                  # translates missing and stale strings only
glot translate --batch          # same, through the Message Batches API: -50 %, asynchronous, resumable
glot check                      # deterministic QA, exit 1 on errors (--format text|json|sarif|github)
```

Every output goes through the [validator](concepts.md#validator) before it is written. Hand-edited translations are never overwritten without `--force`. `glot status` shows coverage per locale and what the next `translate` would do.

## Visual commands need Node.js and Chromium

`glot fit`, `glot measure`, `glot coverage --render` and `glot translate --visual` drive a real browser through the `@glot/visual-qa` helper (Playwright); `glot parity` uses `@glot/agent-qa`. The glot binary itself needs neither. glot starts the helper with `npx -y @glot/visual-qa` (pinned to its own version) unless `glot-visual-qa` is on your PATH, so you only need:

```sh
node --version                   # Node.js >= 20
npx -y playwright install chromium
```

To use a local checkout instead, set `GLOT_VISUAL_QA=/path/to/visual-qa/dist/cli.js` (`GLOT_AGENT_QA` for `parity`).

## Next steps

- Let your coding agent do this while you code: [MCP server](mcp.md).
- Keep it running on every pull request: [GitHub Action](integrations.md#github-action).
- Fix translations that overflow their button: [pixel budgets and `glot fit`](concepts.md#pixel-budgets).
- Edit text in the running app with Alt+click: [`glot dev` + overlay](integrations.md#in-app-overlay).
