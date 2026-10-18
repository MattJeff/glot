# X and LinkedIn drafts

## X (thread)

1/
I open-sourced glot: i18n for SaaS where the LLM never writes to your catalogs unchecked.

Every translation is validated (ICU syntax, CLDR plurals, placeholders, glossary, pixel width) before it is written. MIT, bring your own key.

github.com/MattJeff/glot
[i18nize.gif]

2/
On the demo Next.js app: 117 hardcoded strings.
`glot i18nize` rewrites them to t() calls, turns string concatenation into ICU plurals and wires next-intl.
`glot extract` afterwards: nothing to report.

3/
`glot.lock` hashes source + target per key:
- only missing/stale strings are translated
- a translation someone fixed by hand is never overwritten without --force

850 strings with --batch on Haiku: $0.035.

4/
Alt+click any text in your running app to edit it in every locale. Saved to your JSON, validated, hot-reloaded.
[overlay.gif]

5/
Coding agents get live context through MCP (keys, file:line, UI role, glossary):
claude plugin marketplace add MattJeff/glot

Docs, CLI reference and limits: github.com/MattJeff/glot

## LinkedIn

I just released glot v0.2.1, an open-source (MIT) localization tool for SaaS teams.

The problem it addresses: LLMs translate UI text well, but they also drop placeholders, produce plural forms the target language does not have, and overwrite fixes a human made last week. Teams end up reviewing everything by hand, or shipping English only.

glot puts deterministic code around the model:
• a validator checks every translation (ICU/i18next/MessageFormat 2 syntax, CLDR plurals, placeholders, glossary, pixel width) before it is written; failures are retried, escalated, then rejected;
• a lockfile in Git means only changed strings are translated and hand edits are kept;
• `glot i18nize` converts hardcoded strings in React/Next apps (117 → 0 on our demo app);
• an MCP server lets Claude Code, Cursor or Codex add and translate strings while you code.

You use your own LLM key. On Haiku with batch mode, 850 strings cost $0.035.

Repo, docs and install (npm, Homebrew, Claude Code plugin): https://github.com/MattJeff/glot

Feedback from anyone running a multilingual product is very welcome.
