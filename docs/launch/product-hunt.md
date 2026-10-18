# Product Hunt draft

**Name**: glot

**Tagline** (60 chars max):

> Your agent localizes your app. Code refuses what's broken.

Alternatives: `AI translations refused until they fit the button` · `Localization your coding agent can't break`

**Topics**: Developer Tools, Open Source, Artificial Intelligence

**Description** (260 chars max):

> Your coding agent moves strings to keys and translates them. glot checks every LLM output (placeholders, CLDR plurals, glossary, pixel width measured in your app) and refuses it, with the reason, until the retry is valid. CLI + MCP server, MIT.

**Gallery** (in order):

1. `web/public/launch/refusal.gif`: glot refuses three bad translations and writes the valid retries (scripted LLM; say so in the caption, or re-record with a real model first)
2. `web/public/launch/fit-before-after.png`: German overflow before / after `glot fit` (real model)
3. `web/public/launch/i18nize.gif`: extract → i18nize → extract on the demo app
4. `web/public/launch/overlay.gif`: Alt+click a label in the running app, edit the German, saved and validated

**Maker's first comment**:

---

Hi Product Hunt! I'm the maker of glot, a young open-source project.

LLMs translate UI copy well enough, but they still drop Polish plural forms, rename `{name}` to `{nom}`, and write German labels twice as wide as the button. glot puts deterministic code between the model and your catalogs:

1. Your coding agent (Claude Code, Cursor, Codex, through the MCP server) or the CLI moves hardcoded strings to keys and translates the delta.
2. Every output is checked: syntax, placeholders, CLDR plural categories, glossary, and width against the box measured in your running app. A refused string goes back to the model with the exact reason; nothing invalid is written.
3. `glot.lock` keeps hand edits from being overwritten.

The pixel budget and the validator in the loop are the parts I think are new. The self-hosted dashboard and the growth features (insights, experiments) are experimental and labelled as such.

MIT and free; you pay your LLM provider directly. I'd like to hear what a translation tool has broken for you before.

---

Notes:

- Launch at 00:01 PT. Have the GitHub README, `/docs` and the release (v0.2.1) live before.
- Product Hunt is not where developers evaluate a CLI: treat it as secondary to HN and Reddit.
