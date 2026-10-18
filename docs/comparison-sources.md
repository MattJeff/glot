# Comparison sources

Every cell of the landing page comparison table (`web/app/[locale]/page.tsx`), with the public page it is based on.
All pages were checked on **2026-10-18**. Vendors change plans and features often: if a cell looks wrong, open an issue with a link.

Legend: **Yes** = documented. **Partial** = limited, plan-dependent or through an integration. **No** = we found no such feature in the vendor's public docs (it may exist privately or on request).

## Price (entry point)

| Tool | Value | Source |
|---|---|---|
| Lokalise | From $149/mo (Explorer), no free plan, 14-day trial | https://lokalise.com/pricing |
| Crowdin | Paid plans (amounts not shown in page text), 14-day trial (Team) / 30-day (Enterprise), free for open-source non-profit projects | https://crowdin.com/pricing |
| Lingo.dev | CLI free with your own LLM key; hosted Sandbox $0 base + usage, Production $99/mo base + usage | https://lingo.dev/en/pricing, https://docs.lingo.dev/byok |
| Tolgee | Free cloud tier (30k words, 3 seats), free self-hosted (up to 10 seats), cloud from €58/mo billed annually | https://tolgee.io/pricing, https://docs.tolgee.io/platform/self_hosting/getting_started |

## Open source

| Tool | Value | Source |
|---|---|---|
| Lokalise | No. Platform is proprietary; only tools such as the CLI are open (BSD-3-Clause) | https://github.com/lokalise/lokalise-cli-2-go |
| Crowdin | No. Platform is proprietary; the CLI is MIT | https://github.com/crowdin/crowdin-cli |
| Lingo.dev | Partial. CLI/compiler repo is Apache-2.0, the localization engine is a hosted service | https://github.com/lingodotdev/lingo.dev |
| Tolgee | Partial (open core). Apache-2.0 except `ee/` directories under a separate license | https://github.com/tolgee/tolgee-platform/blob/main/LICENSE |

Self-hosting (not a table row): Tolgee documents a free self-hosted version (Docker/Java). We found no official self-hosted offering for Lokalise or Lingo.dev's engine; for Crowdin we found only third-party claims of on-premise Enterprise, not an official page.

## Bring your own LLM key

| Tool | Value | Source |
|---|---|---|
| Lokalise | No. Docs describe Lokalise-managed models (routing, choice of GPT/Claude in AI profiles), no customer API key | https://docs.lokalise.com/en/articles/12292275-ai-frequently-asked-questions |
| Crowdin | Yes. "Use my own API key" for AI providers incl. OpenAI and Anthropic | https://support.crowdin.com/crowdin-ai/ |
| Lingo.dev | Yes. `provider` block in `i18n.json` (OpenAI, Anthropic, Google, Mistral, OpenRouter, Ollama) | https://docs.lingo.dev/byok |
| Tolgee | Yes. Custom LLM providers per organization (cloud) or server config (self-hosted) | https://docs.tolgee.io/platform/projects_and_organizations/llm-providers |

## Deterministic validation (placeholders, ICU, plurals)

| Tool | Value | Source |
|---|---|---|
| Lokalise | Partial. QA checks for placeholders, HTML, etc., but "currently QA checks are not supported for plural keys"; ICU not mentioned | https://docs.lokalise.com/en/articles/2564656-qa-checks |
| Crowdin | Yes. Placeholders mismatch and ICU syntax checks (incl. missing `other`), enabled by default | https://support.crowdin.com/qa-checks/ |
| Lingo.dev | No. No deterministic placeholder/ICU check found in public docs (only AI quality review, billed per run) | https://lingo.dev/en/pricing |
| Tolgee | Partial. Placeholder, HTML and ICU syntax checks per plural form, but only on advanced plans / licensed self-host | https://docs.tolgee.io/platform/translation_process/qa_checks |

## Git as source of truth (lockfile)

| Tool | Value | Source |
|---|---|---|
| Lokalise | Partial. Platform is the source of truth; GitHub app / GitHub Actions open PRs with files | https://docs.lokalise.com/integrations/github, https://developers.lokalise.com/docs/github-actions |
| Crowdin | Partial. GitHub integration syncs files and opens PRs from an `l10n_` service branch | https://support.crowdin.com/github-integration/ |
| Lingo.dev | Yes. Lockfile with SHA-256 fingerprints of source content, `--frozen` for CI | https://lingo.dev/en/cli/fundamentals/i18n-lock-lockfile |
| Tolgee | Partial. Platform is the source of truth; CLI push/pull to files | https://docs.tolgee.io/tolgee-cli/push-pull-strings |

## MCP server for coding agents

| Tool | Value | Source |
|---|---|---|
| Lokalise | Yes. Cloud-hosted MCP server, OAuth | https://lokalise.com/product/mcp-server/ |
| Crowdin | Yes. Hosted at `mcp.crowdin.com` | https://support.crowdin.com/developer/crowdin-mcp-server/ |
| Lingo.dev | Yes. Gives assistants access to the localization engine configuration | https://lingo.dev/en/mcp |
| Tolgee | Yes. Built into the platform (`/mcp/developer`), listed as `io.github.tolgee/tolgee` | https://docs.tolgee.io/platform/integrations/mcp_server/about |

Note: these servers expose the vendor platform (projects, keys, engine config). glot's MCP server also exposes local context (file:line, UI role) from your repo.

## In-context editing in your app

| Tool | Value | Source |
|---|---|---|
| Lokalise | Yes (LiveJS, Lokalise Expert; not in Vantage) | https://docs.lokalise.com/en/articles/1592525-livejs-web-in-context-editor |
| Crowdin | Yes (In-Context overlay) | https://support.crowdin.com/in-context-localization |
| Lingo.dev | No. Not found in public docs | https://lingo.dev |
| Tolgee | Yes (Alt+click in-context editing, Chrome extension for production) | https://docs.tolgee.io/blog/in-context-production |

## Rendered pixel-budget fitting (visual QA)

| Tool | Value | Source |
|---|---|---|
| Lokalise | Partial. Per-key character limit, not rendered pixel width | https://docs.lokalise.com/en/articles/1929443-character-limit |
| Crowdin | Partial. Per-string max length | https://support.crowdin.com/qa-checks/ |
| Lingo.dev | No. Not found in public docs | https://lingo.dev |
| Tolgee | Partial. Optional character limit per key ("Key length limit" QA check) | https://docs.tolgee.io/platform/translation_keys/keys |
