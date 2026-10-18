# Show HN draft

**Title** (80 chars max):

> Show HN: Glot – open-source i18n for SaaS where every LLM translation is validated

Alternative: `Show HN: Glot – a CLI + MCP server that internationalizes a React app and checks every translation`

**URL**: https://github.com/MattJeff/glot

**Text** (post as the first comment if the URL field is used):

---

Hi HN, I built glot because every "AI translation" tool I tried would happily write `{count, plural, one {# article}}` into a Polish catalog (missing `few`/`many`), translate a placeholder name, or overwrite the fix a colleague made by hand last week.

glot is a Rust CLI (MIT) that sits next to the i18n library you already use (next-intl, i18next, FormatJS). The LLM is never trusted on its own:

- Every output goes through a deterministic validator before it is written: ICU / i18next / MessageFormat 2 syntax, placeholders, CLDR plural categories of the target locale, rich-text tags, glossary terms, length. A failure is retried with the issues in the prompt, then escalated to a fallback model. If it still fails, nothing is written.
- `glot.lock` hashes source and target per key. Only missing or stale strings are translated, and a translation edited by hand is never overwritten without `--force`.
- `glossary.toml` lives in the repo and is enforced by the validator.

It also does the part before translation. On the demo app in the repo (a small Next.js invoicing SaaS with everything hardcoded), `glot extract` finds 117 hardcoded strings; `glot i18nize` rewrites the code to `t()` calls (including turning `"You have " + n + " overdue invoices"` into an ICU plural) and wires next-intl; `extract` then reports none left. It supports Next (App and Pages Router), React Router 7/8, Astro with React islands and Vite + React.

Other pieces, all in the same repo:

- `glot mcp`: an MCP server so Claude Code, Cursor or Codex get your keys, where each is used (file:line, UI role, component), the glossary and notes, and add strings through `propose_string` → `add_string` instead of hardcoding them.
- `glot measure` records each key's pixel box from the running app; the validator then shapes translations with rustybuzz (no browser) and flags the ones that will not fit. `glot fit` shortens what overflows and re-renders until it fits.
- `glot dev` + a 6 kB overlay: Alt+click any text in your running app to edit it in every locale, validated on save.
- An optional self-hosted dashboard (Rust/axum + Postgres + Next.js). The dashboard itself is translated by glot in CI.

Costs: you bring your own key. With Haiku as the default model, translating 850 strings with `--batch` (Anthropic's Message Batches API, -50 %) cost $0.035. The system prompt with the glossary is a cached prefix, so repeated batches pay ~10 % of it. `glot translate --dry-run` prints an estimate first; an early version underestimated (real $0.071 vs a $0.060 "max") because it ignored that every retry re-sends the system prompt, which is now modeled.

Known limits: code rewriting is for JS/TS/JSX only; the Astro setup shares one i18next instance (fine for static builds, not for concurrent SSR); visual features need your app running locally with Playwright; the on-device fallback for unshipped languages uses Chrome's Translator API, so other browsers fall back to the source language.

Install: `npx glot-i18n init`, `brew install mattjeff/glot/glot`, or the Claude Code plugin (`claude plugin marketplace add MattJeff/glot`).

I would love feedback on the validator rules and on what breaks with your stack.

---

Notes for posting:

- Post Tuesday–Thursday, 8–10am US Eastern. Stay in the thread for the first 3 hours.
- Do not ask for upvotes anywhere. Reply to every technical question with specifics and links to code.
- Likely questions to prepare for: why not Lingo.dev / Tolgee (point to `docs/comparison-sources.md`), privacy (what is sent to the LLM: strings + context, not source files, no telemetry), quality vs DeepL, why Rust.
