# Agent skill

glot ships a portable skill in the open [Agent Skills](https://agentskills.io/specification) format: a `glot/` folder with a `SKILL.md` (name + description frontmatter, instructions), `scripts/glot.sh` (runs `glot`, or `npx -y glot-i18n` when it is not installed) and `references/` (command list, provenance). Agents load the description at startup and the instructions only when a task needs them: "make this app multilingual", "add a button label", "translate to German".

Source: [`integrations/skills/glot`](https://github.com/MattJeff/glot/tree/main/integrations/skills/glot).

## Install

```sh
# skills.sh: asks which agents to install for (Claude Code, Codex, Cursor, Gemini CLI, Copilot…)
npx skills add MattJeff/glot --skill glot
```

`glot init` also writes a project-specific copy (your locales and catalogs) to `.claude/skills/glot/` and to the shared `.agents/skills/glot/`.

By hand, copy the folder to the directory your agent reads:

| Agent | Project | Personal |
|---|---|---|
| Shared convention (Codex, Cursor, VS Code Copilot; Gemini CLI fallback) | `.agents/skills/glot/` | `~/.agents/skills/glot/` |
| Claude Code | `.claude/skills/glot/`, or the [plugin](integrations.md#claude-code) | `~/.claude/skills/glot/` |
| Gemini CLI | `.gemini/skills/glot/` | `~/.gemini/skills/glot/` |
| GitHub Copilot | `.github/skills/glot/` | `~/.copilot/skills/glot/` |

Paths move between agent releases: check your agent's docs if the skill does not show up.

## What the agent does with it

1. No `glot.toml`: asks which languages, then runs `glot localize --yes --targets fr,de`.
2. Existing project: `glot localize --yes` (init if needed → i18nize → translate → check → status). It is idempotent, so the agent reruns it after any UI change. `--dry-run` shows the plan and the cost first.
3. New UI text: `propose_string` + `add_string` through the [MCP server](mcp.md), or `glot add`, then translate and check.
4. `glot check` must exit 0 before the agent says it is done.

## App builders and other agents

Lovable, Bolt, v0 and similar builders do not load skills. Paste a prompt instead:

```sh
glot agent-prompt               # for a coding agent (CLAUDE.md, AGENTS.md, a chat)
glot agent-prompt --for lovable # for an app builder: keep code translation-ready, translate outside
glot localize --print-agent-prompt
```

The prompt is generated from your stack (next-intl, react-i18next, react-intl, vue-i18n, Lingui, or none yet) and from `glot.toml` (source locale, targets, catalog paths).
