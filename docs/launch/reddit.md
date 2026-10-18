# Reddit drafts

Read each subreddit's self-promotion rules the day you post. Post from an account with history, one subreddit per day, and answer comments for the first hours. Never cross-post the same text.

## r/nextjs

**Title**: Open-source CLI: your coding agent localizes your Next.js app, and code refuses translations that break plurals, placeholders or the button width

**Body**:

I kept doing the same chore on Next.js projects: move every string into `messages/en.json`, wire next-intl, translate, then fix what the LLM got wrong (Polish plurals, renamed placeholders, German labels that overflow). glot (MIT, Rust CLI + MCP server, young project) automates that and refuses to write what it can't verify.

- `glot i18nize` rewrites hardcoded strings to `useTranslations`/`getTranslations`, turns `"You have " + n + " invoices"` into an ICU plural and wires next-intl (cookie routing by default, `--routing prefix` for `/fr/...`). `--dry-run` shows the diff first.
- `glot translate` translates only what changed with your own key. Every output goes through a validator: ICU syntax, placeholders, CLDR plural categories, glossary, and width against the box `glot measure` recorded from your running app. A refused string is retried with the exact reason; nothing invalid is written. The README GIF shows three refusals (LLM scripted to fail on cue; the binary and messages are real).
- `glot.lock` keeps hand edits from being overwritten. `glot mcp` lets Claude Code or Cursor add strings through the same checks.

Limits: JS/TS/JSX only; key names come from one LLM call, so review them in the diff; the self-hosted dashboard is experimental.

Repo: https://github.com/MattJeff/glot. I'd like to know what breaks on your codebase.

## r/webdev

**Title**: I made LLM translations go through a validator that refuses broken plurals, placeholders and text too wide for its button

**Body**:

glot is an open-source CLI (MIT) for adding languages to a React-based web app without trusting the model blindly. It's young; here is what it does today:

- Your coding agent or the CLI moves hardcoded strings to keys (Next.js, React Router 7/8, Astro with React islands, Vite + React).
- Each LLM translation is checked by code before it is written: syntax, plural forms per language, placeholders, glossary terms, and the rendered width against the box measured in your running app. Failures go back to the model with the reason, then to a fallback model; anything still invalid is not written.
- Only changed strings are translated; hand edits are protected by a lockfile.

Bring your own Anthropic/OpenAI key or a local model through any OpenAI-compatible endpoint. Catalogs stay plain JSON in your repo; no account.

Demo GIF and docs: https://github.com/MattJeff/glot

Happy to answer questions about how the validation works.
