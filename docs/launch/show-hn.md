# Show HN draft

**Title** (80 chars max):

> Show HN: Glot – your coding agent localizes your app, code refuses what's broken

Alternative: `Show HN: Glot – LLM translations refused until they fit the button and keep the plurals`

**URL**: https://github.com/MattJeff/glot

**Text** (post as the first comment if the URL field is used):

---

Hi HN. glot is a young open-source project (MIT, Rust CLI + MCP server) built on one idea: your coding agent can do the localization work, as long as deterministic code decides what gets written.

The README GIF shows the loop. The LLM proposes a Polish plural without `few`/`many`, renames `{name}` to `{nom}` in French, and gives a German button label that renders 276 px wide in a 120 px box. glot refuses all three with the exact reason ("missing plural categories required by pl: few, many", "placeholder `{nom}` does not exist in the source", "must fit in one line of 120px at Geist 14px/400; it renders 276px wide"), sends those reasons back to the model, checks the retries and writes only those. In that recording the LLM is scripted to fail on cue (a good model rarely does on demand); the binary and every message are real, and the script in the repo reruns it with a real model doing the retries.

Two things I have not seen elsewhere:

- **Pixel budgets.** `glot measure` records each key's box (width, lines, font) from your running app. Translations are then shaped with that font file (rustybuzz, no browser) and refused if they don't fit, before they are written. `glot fit` shortens what already overflows and re-renders until it fits.
- **The validator in the loop**, not after it: ICU / i18next / MessageFormat 2 syntax, placeholders, CLDR plural categories of the target locale, glossary terms and width are checked on every model output. A failure is retried with the issues in the prompt (2 retries, then a fallback model). If it still fails, nothing is written and the key is reported.

Around that: `glot.lock` hashes source and target per key, so only the delta is translated and a hand edit is never overwritten without `--force`. `glot i18nize` moves hardcoded strings to keys and rewrites JSX for next-intl or react-i18next (117 hardcoded strings → 0 on the demo app in the repo). `glot mcp` lets Claude Code, Cursor or Codex see your keys, where they are used and the glossary, and add strings through the same validator.

What it is not (yet): code rewriting is JS/TS/JSX only; pixel budgets need your app running locally with Playwright; the self-hosted server and dashboard are experimental; SEO, database content, emails/images and task parity live under `glot labs` because they are newer and less used.

Bring your own key (Anthropic, OpenAI, any OpenAI-compatible endpoint). `npx glot-i18n init` to try it, `--provider mock` to try it without a key.

I'd love feedback on the validator rules and on what breaks with your stack.

---

Notes for posting:

- Post Tuesday–Thursday, 8–10am US Eastern. Stay in the thread for the first 3 hours.
- Do not ask for upvotes anywhere. Reply to every technical question with specifics and links to code.
- Say "scripted LLM" before anyone asks; link `docs/launch/record/refusal.sh`. Re-record with a real model for the retries before posting if you can (see the checklist).
- Likely questions: why not Lingo.dev / Tolgee (point to `docs/comparison-sources.md`), privacy (strings + context are sent to the LLM, not source files, no telemetry), quality vs DeepL, why Rust, cost (`glot translate --dry-run` prints an estimate; quote a number only with its setup written down).
