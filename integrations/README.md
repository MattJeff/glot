# Integrations

Everything that puts glot where developers and AI agents already are. Nothing here is published yet; the checklist at the end lists what to do per channel.

| Path | What |
|---|---|
| [`install.sh`](install.sh) | `curl \| sh` installer, SHA-256 verified |
| [`../packages/npm-glot`](../packages/npm-glot) | npm package `glot-i18n` (`npx glot-i18n …`) |
| [`homebrew/glot.rb`](homebrew/glot.rb) | Formula for the `MattJeff/homebrew-glot` tap |
| [`claude-code/`](claude-code) | Claude Code plugin marketplace + `glot` plugin (skill, `/glot:i18nize`, `/glot:translate`, MCP) |
| [`agents/`](agents) | `AGENTS.md` (Codex & co.), Cursor `.mdc`, Windsurf rule |
| [`registries/`](registries) | MCP registry `server.json`, `glama.json`, `smithery.yaml` |
| [`ai-builders.md`](ai-builders.md), [`ai-builders-prompt.md`](ai-builders-prompt.md) | Lovable / Bolt / v0 / Replit guide and knowledge snippet |
| [`workflows/glot-onboard.yml`](workflows/glot-onboard.yml) | One-click "Translate this app" workflow (init + i18nize + translate → PR) |
| [`badge/glot-badge.sh`](badge/glot-badge.sh) | "translated by glot · N languages · X%" shields.io endpoint |

## Coding agents

`glot init` already writes per-project rules (`CLAUDE.md`, `AGENTS.md`, `.claude/skills/glot`, `.cursor/rules/glot.mdc`, `.mcp.json`). The files below are for **before** a project uses glot, so the agent reaches for it when someone asks for a multilingual app.

- **Claude Code**: `claude plugin marketplace add MattJeff/glot` then `claude plugin install glot@glot` (needs [`claude-code/.claude-plugin/marketplace.json`](claude-code/.claude-plugin/marketplace.json) at the repo root, see checklist). Adds the `glot` skill, `/glot:i18nize [paths]`, `/glot:translate [locales]` and the MCP server. MCP only: `claude mcp add glot -- npx -y glot-i18n mcp`.
- **Cursor**: copy [`agents/cursor/glot.mdc`](agents/cursor/glot.mdc) to `.cursor/rules/`. MCP: add `{"mcpServers":{"glot":{"command":"npx","args":["-y","glot-i18n","mcp"]}}}` to `.cursor/mcp.json`.
- **Codex** (and any agent reading `AGENTS.md`): append [`agents/AGENTS.md`](agents/AGENTS.md) to your `AGENTS.md`. MCP: `codex mcp add glot -- npx -y glot-i18n mcp`.
- **Windsurf**: copy [`agents/windsurf/glot.md`](agents/windsurf/glot.md) to `.windsurf/rules/`. MCP: same JSON as Cursor in `~/.codeium/windsurf/mcp_config.json`.

The MCP server is stdio only (`glot mcp`) and must start inside a project with `glot.toml` (run `glot init` first).

## Badge

```sh
integrations/badge/glot-badge.sh                # prints {"schemaVersion":1,"label":"translated by glot","message":"3 languages · 96%",...}
integrations/badge/glot-badge.sh --push         # commits it as glot-badge.json to an orphan `glot-badge` branch
```

```md
![glot](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/OWNER/REPO/glot-badge/glot-badge.json)
```

Needs `glot`, `jq`, `git`, and a public repo (shields.io fetches the raw file). In CI, run it after the glot action on pushes to the default branch:

```yaml
on: { push: { branches: [main] } }
permissions: { contents: write }
jobs:
  badge:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: npm install -g glot-i18n
      - run: curl -fsSL https://raw.githubusercontent.com/MattJeff/glot/main/integrations/badge/glot-badge.sh | sh -s -- --push
```

For the action itself: add an input `badge: "false"` and, when true and `github.event_name == 'push'`, a last step running `glot-badge.sh --push` from `${{ github.action_path }}/../integrations/badge/`.

## Release checklist (per glot version)

1. **npm** (`glot-i18n` is free; `glot` and `glot-cli` are taken; fallbacks: `@mattjeff/glot`, `getglot`, `glotai`): in `packages/npm-glot`, `scripts/sync-checksums.sh vX.Y.Z`, set `version` to `X.Y.Z`, `npm publish`. The package downloads `glot-<target>` from the `vX.Y.Z` release, so the release must exist first. The repo must be public (release downloads are anonymous).
2. **Homebrew**: create `MattJeff/homebrew-glot` with `Formula/glot.rb` from [`homebrew/glot.rb`](homebrew/glot.rb); bump the four URLs + sha256. Users: `brew install mattjeff/glot/glot`.
3. **install.sh**: works as is from `main`. Better: add a `SHA256SUMS` asset to the release workflow (`sha256sum glot-* > SHA256SUMS`); until then the script verifies against the digest GitHub's API records for each asset.
4. **MCP registry**: after the npm publish (its `mcpName` must equal `io.github.MattJeff/glot`), bump both versions in [`registries/server.json`](registries/server.json), then `mcp-publisher login github && mcp-publisher publish registries/server.json`.
5. **Glama**: copy [`registries/glama.json`](registries/glama.json) to the repo root, submit the repo on glama.ai.
6. **Smithery**: copy [`registries/smithery.yaml`](registries/smithery.yaml) to the repo root and publish from smithery.ai (check its current config format first).
7. **Claude Code plugin**: copy `claude-code/.claude-plugin/marketplace.json` to `.claude-plugin/marketplace.json` at the repo root and change the plugin `source` to `"./integrations/claude-code/glot"`; bump `glot/.claude-plugin/plugin.json` `version` and the pinned `glot-i18n@X.Y.Z` in `glot/.mcp.json`. Check with `claude plugin validate .`. Optionally submit to Anthropic's plugin directory.
8. **glot-onboard.yml**: pin `npm install -g glot-i18n@X.Y.Z` once a release ships `i18nize`.
