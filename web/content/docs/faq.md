# FAQ & costs

## What does glot cost?

The CLI, the validator and the MCP server are MIT-licensed and free: no seats, no per-word pricing. You pay your LLM provider directly with your own key. glot cloud (dashboard, review, GitHub sync, copy experiments) has a free plan and paid Pro and Team plans: see [pricing](/pricing). Self-hosting the server costs whatever your Postgres and two containers cost, with no limits.

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

## Which environment variables does glot read?

| Variable | Used by | |
|---|---|---|
| `ANTHROPIC_API_KEY` | `translate`, `review`, `i18nize`, MCP | Key for `provider = "anthropic"` (name configurable with `llm.api_key_env`) |
| `OPENAI_API_KEY` | same | Key for `openai` / `openai-compatible` (optional for local servers) |
| `ANTHROPIC_WORKSPACE_ID` | same | Sent as `anthropic-workspace-id`; required by organization-level user keys (`sk-ant-usr-…`) |
| `GLOT_TOKEN` | `push`, `pull` | Server API token (name configurable with `server.token_env`) |
| `GLOT_DEV_TOKEN` | `dev` | Fixed token for the overlay server (else `--token`, else random) |
| `GLOT_VISUAL_QA` | `fit`, `measure`, `coverage --render`, `translate --visual` | Path to a `visual-qa` CLI to use instead of `npx -y @glot-i18n/visual-qa` |
| `GLOT_AGENT_QA` | `parity` | Same for `agent-qa` |
| `DATABASE_URL` | `db` | Postgres URL when `--database-url` is not given |

Server variables (`GLOT_SECRET_KEY`, `GLOT_BIND`, `GLOT_WEB_ORIGIN`, rate limits…) are listed in [Self-hosting](self-hosting.md). The installer reads `GLOT_VERSION`, `GLOT_INSTALL_DIR` and `GLOT_DOWNLOAD_BASE`.

## Does my code leave my machine?

The CLI sends to your LLM provider the strings to translate and their context: key, neighbors, notes, glossary terms, UI role, component name and file path. Not your source files. With `--visual`, screenshot crops of your running app are sent too. Nothing goes to glot's authors; there is no telemetry.

## Can I edit translations by hand?

Yes, anywhere: in the JSON, with `glot set fr key "…"`, in the dashboard or with the in-app overlay. The lockfile notices the edit and `translate` will not overwrite it without `--force`. Hand edits are validated too (`glot check`).

## Does glot add a runtime dependency?

No. Your app keeps using next-intl, i18next or FormatJS with plain catalog files: JSON, JS dictionaries, Apple `.xcstrings` and `.strings`, Android `strings.xml`, Flutter ARB, gettext PO or YAML. The only runtime packages are opt-in: [`@glot-i18n/runtime`](runtime.md) (on-device fallback), `@glot-i18n/overlay` (dev only) and `@glot-i18n/insights`.

## How is this different from Lokalise, Crowdin or Lingo.dev?

Git is the source of truth (no sync to a hosted editor), every LLM output is validated deterministically before it is written, coding agents get live context through MCP, and translations are fitted to the pixels they actually have. The comparison on the [localization page](/i18n#compare) links every cell to its public source.
