# FAQ & costs

## What does glot cost?

glot is MIT-licensed: no seats, no per-word pricing, no hosted plan. You pay your LLM provider directly with your own key. Self-hosting the optional server costs whatever your Postgres and two containers cost.

## How much do translations cost in LLM usage?

With the default model (`claude-haiku-5-5`, $0.10 per million input tokens and $0.50 per million output tokens), translating **850 strings with `--batch` cost $0.035** in our run. Your number depends on string length, the number of target locales, the glossary size, retries and the model. Always check first:

```sh
glot translate --dry-run
```

The estimate uses the real prompts, the expected output (plural forms of each target locale), `--visual` images, the judge if configured, and retries/escalations weighted by their probability. It prints an expected value and a maximum (all retries, escalation on 30 % of items, 1.3× tokenizer variance, no cache hits).

## What makes it cheaper?

- **Delta only.** The [lockfile](concepts.md#lockfile) skips everything already translated. A re-run with no source change costs $0.
- **Prompt caching.** The system prompt (rules + the locale's glossary) is the same for every batch of a locale. It is sent as a cached prefix, so repeated batches pay ~10 % of its price (Anthropic; OpenAI caches the same prefix automatically).
- **`--batch`.** The first pass goes through Anthropic's Message Batches API: -50 %, asynchronous (minutes to hours). The batch id is stored in `.glot/batch.json`, so rerunning the same command resumes it instead of paying twice. Retries and escalations run as direct calls.
- **A cheap default, an expensive fallback.** Haiku translates; `fallback_model` (Sonnet) is only used when a string fails validation after a retry.

## How do I cap spending?

`llm.budget_usd` in `glot.toml`: `translate` refuses to start when the estimate exceeds it. The server applies the per-project budget to its jobs. The GitHub Action comment shows the cost of each run.

## Which providers and models?

`provider = "anthropic" | "openai" | "openai-compatible" | "mock"`. `openai-compatible` takes a `base_url`, so local models through Ollama work (`base_url = "http://localhost:11434/v1"`). `mock` returns fake translations (`⟦fr⟧ …`) for tests and demos, no key needed. Environment variables: `ANTHROPIC_API_KEY` (or another name via `llm.api_key_env`), `ANTHROPIC_WORKSPACE_ID` for organization-level keys, `OPENAI_API_KEY`.

## Does my code leave my machine?

The CLI sends to your LLM provider the strings to translate and their context: key, neighbors, notes, glossary terms, UI role, component name and file path. Not your source files. With `--visual`, screenshot crops of your running app are sent too. Nothing goes to glot's authors; there is no telemetry.

## Can I edit translations by hand?

Yes, anywhere: in the JSON, with `glot set fr key "…"`, in the dashboard or with the in-app overlay. The lockfile notices the edit and `translate` will not overwrite it without `--force`. Hand edits are validated too (`glot check`).

## Does glot add a runtime dependency?

No. Your app keeps using next-intl, i18next or FormatJS with plain JSON catalogs. The only runtime packages are opt-in: [`@glot/runtime`](runtime.md) (on-device fallback), `@glot/overlay` (dev only) and `@glot/insights`.

## How is this different from Lokalise, Crowdin or Lingo.dev?

Git is the source of truth (no sync to a hosted editor), every LLM output is validated deterministically before it is written, coding agents get live context through MCP, and translations are fitted to the pixels they actually have. The comparison on the [home page](/#compare) links every cell to its public source.
