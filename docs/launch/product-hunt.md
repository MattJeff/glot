# Product Hunt draft

**Name**: glot

**Tagline** (60 chars max):

> Open-source i18n for SaaS, validated before it ships

Alternatives: `Translate your SaaS from your coding agent, safely` · `AI translations with a lockfile and a validator`

**Topics**: Developer Tools, Open Source, Artificial Intelligence, SaaS

**Description** (260 chars max):

> glot internationalizes a React/Next app (117 hardcoded strings → 0 on our demo), translates only what changed with your own LLM key, and validates every output (plurals, placeholders, glossary, pixel width) before writing it. CLI + MCP server, MIT.

**Gallery** (in order):

1. `web/public/launch/i18nize.gif`: extract → i18nize → extract on the demo app
2. `web/public/launch/overlay.gif`: Alt+click a label in the running app, edit the German, saved and validated
3. `web/public/launch/locale-matrix.gif`: every locale side by side in the dashboard
4. `web/public/launch/fit-before-after.png`: German overflow before / after `glot fit` (re-record with a real model first, see checklist)
5. Screenshot of `/docs` (concepts page)

**Maker's first comment**:

---

Hi Product Hunt! I'm the maker of glot.

Most SaaS apps ship in English because internationalizing is a week of moving strings into JSON, and then every LLM translation needs a human to check plurals and placeholders. glot automates both halves and refuses to write anything it cannot verify:

1. `glot i18nize` rewrites hardcoded UI strings to translation calls and wires next-intl or react-i18next (Next.js, React Router, Astro, Vite).
2. `glot translate` translates only missing and changed strings with your own Anthropic/OpenAI/Ollama key. Each output is checked for syntax, CLDR plural forms, placeholders, glossary terms and width; failures are retried, then escalated, then rejected.
3. Your coding agent stays in the loop through an MCP server (Claude Code plugin, Cursor, Codex).

It is MIT and free; you pay your LLM provider directly. For scale: 850 strings with batch mode cost us $0.035 on Haiku.

What I'd like to hear: which stack you'd want next, and what a translation tool has broken for you before.

---

Notes:

- Launch at 00:01 PT. Have the GitHub README, `/docs` and the release (v0.2.1) live before.
- Product Hunt is not where developers evaluate a CLI: treat it as secondary to HN and Reddit.
