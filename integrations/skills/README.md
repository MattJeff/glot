# Portable agent skill

[`glot/`](glot) follows the open [Agent Skills](https://agentskills.io/specification) format (`SKILL.md` with `name` + `description` frontmatter, `scripts/`, `references/`). The same folder works in every agent that reads skills; only the directory differs.

| Agent | Project path | Personal path |
|---|---|---|
| Any agent reading the shared convention (Codex, Cursor, VS Code Copilot, Gemini CLI fallback) | `.agents/skills/glot/` | `~/.agents/skills/glot/` |
| Claude Code | `.claude/skills/glot/` (or the plugin in [`../claude-code`](../claude-code)) | `~/.claude/skills/glot/` |
| Gemini CLI | `.gemini/skills/glot/` | `~/.gemini/skills/glot/` |
| GitHub Copilot | `.github/skills/glot/` | `~/.copilot/skills/glot/` |
| Cursor | `.cursor/skills/glot/` | |

`glot init` already writes a project-specific copy to `.claude/skills/glot/` and `.agents/skills/glot/`.

## Install

```sh
# skills.sh (Vercel "skills" CLI): finds integrations/skills/glot in the repo and asks which agents to install for
npx skills add MattJeff/glot --skill glot

# by hand
mkdir -p .agents/skills && cp -R integrations/skills/glot .agents/skills/
```

skills.sh lists a repo once someone installs from it with the CLI (the install count is the ranking); there is nothing to submit. Keep `name: glot` equal to the folder name, the description under 1024 characters, and validate with [`skills-ref validate integrations/skills/glot`](https://github.com/agentskills/agentskills/tree/main/skills-ref) before a release.
