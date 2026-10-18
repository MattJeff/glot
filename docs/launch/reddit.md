# Reddit drafts

Read each subreddit's self-promotion rules the day you post. Post from an account with history, one subreddit per day, and answer comments for the first hours. Never cross-post the same text.

## r/nextjs

**Title**: I built an open-source CLI that converts a hardcoded Next.js app to next-intl and validates every AI translation

**Body**:

I kept doing the same chore on Next.js projects: move every string into `messages/en.json`, wire next-intl, then translate and fix the plurals the LLM got wrong. So I wrote glot (MIT, Rust CLI).

What it does on an App Router project:

- `glot extract` lists hardcoded strings with file:line and a suggested key (117 on the demo app in the repo).
- `glot i18nize` rewrites them to `useTranslations`/`getTranslations` calls, turns `"You have " + n + " invoices"` into an ICU plural, moves module-level config arrays (nav, plans, FAQ) into hooks so their copy can be translated, and writes `i18n/request.ts`, `proxy.ts`, the plugin and the provider. Cookie routing by default (no `app/[locale]` move), `--routing prefix` if you want `/fr/...` URLs. `--dry-run` shows the whole diff first.
- `glot translate` fills `messages/{locale}.json` with your own key. Every output is validated (ICU syntax, CLDR plural categories, placeholders, tags, glossary) and `glot.lock` keeps hand edits from being overwritten.

Pages Router is supported too (built-in `i18n` + provider). For locales you don't ship, there is an optional `@glot/runtime` that translates on-device with Chrome's Translator API and works with Server Components via a `getGlotTranslations` drop-in.

Limits: JS/TS/JSX only; key names come from one LLM call, so review them in the diff.

Repo: https://github.com/MattJeff/glot. I'd like to know what breaks on your codebase.

## r/webdev

**Title**: Open-source tool to internationalize a web app with an LLM, with a validator so it can't break your plurals and placeholders

**Body**:

glot is a CLI (MIT) for adding languages to a React-based web app without trusting the model blindly.

- Finds hardcoded strings and rewrites them to translation calls (Next.js, React Router 7/8, Astro with React islands, Vite + React).
- Translates only what changed, with your own Anthropic/OpenAI key or a local model through Ollama.
- Checks every translation before writing it: syntax, plural forms per language, placeholders, glossary terms, and whether it fits the button it's in (measured from your running app).
- Alt+click any text in your dev server to edit it in every language.

Cost example: 850 strings in batch mode on Claude Haiku came to $0.035. Catalogs stay plain JSON in your repo; there's no hosted service or account.

Demo GIFs and docs: https://github.com/MattJeff/glot

Happy to answer questions about how the validation works.
