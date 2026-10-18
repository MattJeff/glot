# Integrations

`glot init` already writes per-project rules for agents (`CLAUDE.md`, `AGENTS.md`, `.claude/skills/glot`, `.cursor/rules/glot.mdc`, `.mcp.json`). The integrations below are for **before** a project uses glot, so the agent reaches for it when someone asks for a multilingual app. Source files: [`integrations/`](https://github.com/MattJeff/glot/tree/main/integrations).

## Claude Code

```sh
claude plugin marketplace add MattJeff/glot
claude plugin install glot@glot
```

The plugin adds:

- a `glot` skill, which Claude uses whenever you ask for a multilingual app or new UI text;
- `/glot:i18nize [paths]`: convert hardcoded strings, then translate;
- `/glot:translate [locales]`: translate the delta and check;
- the MCP server (pinned `glot-i18n` version).

MCP only, without the plugin: `claude mcp add glot -- npx -y glot-i18n mcp`. See [MCP server](mcp.md) for the tools.

## Cursor

Copy [`agents/cursor/glot.mdc`](https://github.com/MattJeff/glot/blob/main/integrations/agents/cursor/glot.mdc) to `.cursor/rules/`, and add the MCP server to `.cursor/mcp.json`:

```json
{ "mcpServers": { "glot": { "command": "npx", "args": ["-y", "glot-i18n", "mcp"] } } }
```

Windsurf: copy [`agents/windsurf/glot.md`](https://github.com/MattJeff/glot/blob/main/integrations/agents/windsurf/glot.md) to `.windsurf/rules/`; same MCP JSON in `~/.codeium/windsurf/mcp_config.json`.

## Codex

Append [`agents/AGENTS.md`](https://github.com/MattJeff/glot/blob/main/integrations/agents/AGENTS.md) to your `AGENTS.md` (also read by other agents that follow the convention), then:

```sh
codex mcp add glot -- npx -y glot-i18n mcp
```

## Lovable, Bolt, v0, Replit

AI app builders generate English-only apps with hardcoded strings. Two steps:

1. **Before generating**: paste the [knowledge snippet](https://github.com/MattJeff/glot/blob/main/integrations/ai-builders-prompt.md) into the project instructions (Lovable: Project settings → Knowledge; Bolt: project prompt; v0: project instructions; Replit: `replit.md`). Every string then goes through next-intl or react-i18next from the first prompt.
2. **After generating**: push to GitHub and run the **Translate this app** workflow ([`glot-onboard.yml`](https://github.com/MattJeff/glot/blob/main/integrations/workflows/glot-onboard.yml)). One run installs the i18n library if missing, runs `glot init`, `glot i18nize` and `glot translate` (if `ANTHROPIC_API_KEY` is set as a secret) and opens a pull request with coverage and the `glot check` result.

Lovable commits straight to `main`, so PR-triggered workflows do not see its edits: re-run the onboard workflow to catch up. Replit has a shell, so run the CLI directly:

```sh
npx -y glot-i18n init --yes --targets fr,es,de
npx -y glot-i18n i18nize --yes
npx -y glot-i18n translate --yes
```

Full per-tool guide: [`ai-builders.md`](https://github.com/MattJeff/glot/blob/main/integrations/ai-builders.md).

## GitHub Action

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

It validates catalogs (inline annotations), translates the delta, commits the catalogs, `glot.lock` and `.glot/layout.json` to the PR branch (nothing else), and keeps one PR comment up to date with coverage, issues and cost. The binary is pinned to the action release and verified against the release's `SHA256SUMS`.

| Input | Default | |
|---|---|---|
| `command` | `check` | `check`, `translate` or `both` |
| `working-directory` | `.` | Directory containing `glot.toml` |
| `version` | pinned to the action release | glot release tag, or `latest`; skipped when `glot` is on PATH |
| `anthropic-api-key` | | Exported as `ANTHROPIC_API_KEY` |
| `openai-api-key` | | Exported as `OPENAI_API_KEY` |
| `llm-api-key` | | Exported under `llm.api_key_env` (else the provider's default variable) |
| `anthropic-workspace-id` | | Exported as `ANTHROPIC_WORKSPACE_ID` (org-level keys `sk-ant-usr-…`) |
| `comment` | `true` | Post or update the summary comment |
| `fail-on-error` | `true` | Fail the job when `glot check` reports errors |
| `github-token` | `github.token` | Used to comment |
| `token` | | PAT or GitHub App token used to push. Commits pushed with the default `GITHUB_TOKEN` do not trigger CI runs; set this if required checks must run on the translation commit |

Permissions: `contents: write` to push (`contents: read` is enough for `command: check`), `pull-requests: write` to comment. Details in [`action/README.md`](https://github.com/MattJeff/glot/blob/main/action/README.md).

Output: `check-exit-code`.

## In-app overlay

Edit translations in your running app. Start the local server in the project, then load the overlay in development:

```sh
glot dev                 # binds 127.0.0.1:4317, prints a token
```

```tsx
// app/layout.tsx (Next.js App Router)
import { GlotOverlay } from "@glot/overlay/react";
{process.env.NODE_ENV === "development" && <GlotOverlay />}
```

Or without installing anything: `<script src="http://127.0.0.1:4317/overlay.js" defer></script>`.

**Alt+click** any text: a popover shows its key, its value in every locale (editable, validated on save), open issues, whether it overflows, and AI actions (Suggest, Shorten, More formal, Translate all). Text without a key can be extracted to a new key. Saves go to your catalog files, so HMR reloads the page. **Alt+Shift+G** toggles inspect mode. The overlay is ~6 kB gzipped and never starts in production.

## Coverage badge

```sh
integrations/badge/glot-badge.sh --push   # commits glot-badge.json to an orphan `glot-badge` branch
```

```md
![glot](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/OWNER/REPO/glot-badge/glot-badge.json)
```

Renders as "translated by glot · N languages · X%".
