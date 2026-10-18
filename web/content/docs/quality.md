# Quality, review queue and cost

glot writes a translation only after the validator accepts it. Some translations still need a person to look at them: low-confidence ones, and the legal, payment and marketing copy you never want to ship unread. This page covers the quality score, the review queue, the translation memory (which brings the cost of repeated strings to zero) and local models.

## Quality score

Every translation `glot translate` writes gets a score from 0 to 100, stored as `score` in `glot.lock`:

- the **judge** score when `llm.judge_model` is set (an MQM-style LLM review: 100 minus 2 per minor, 10 per major and 25 per critical error);
- otherwise 100;
- minus **15 per validator warning** still present on the written value: `too_long` (length ratio or `max_length`), `too_wide` (measured pixel budget), glossary warnings, whitespace changes.

For a back-translation check of a written string, run `glot review`. Its result is not part of the score.

## Review queue

```toml
[qa]
review_threshold = 85          # score under this -> needs a human
score_threshold = 80           # judge warning + escalation to fallback_model (see below)

[qa.risk]                      # always reviewed, whatever the score
legal   = ["legal.*", "terms.*"]
payment = ["billing.*", "checkout.*"]
marketing = ["landing.hero.*"]
```

A key can also carry its risk tag in its note: `glot add checkout.pay "Pay now" --note "Pay button\nrisk: payment"`.

A routed translation is still written, so your app keeps working. Its `glot.lock` entry gets `"needs_review": "<reason>"` (for example `score 72 < 85` or `risk: legal`) until someone approves it:

```bash
glot queue                                  # list pending entries (--json)
glot queue approve fr legal.terms           # accept as is
glot queue edit fr legal.terms "J'accepte…" # replace (validated) and approve
glot queue export --format xliff -o review.xlf   # or --format csv
glot queue import review.xlf                # unchanged targets approved, edited ones set and approved
```

The CSV export has the columns `locale,key,source,target,reason`. A reviewer edits only `target`. The XLIFF export is XLIFF 1.2 with one `<file>` per locale, `state="needs-review-translation"` and the reason in a `<note>`, which CAT tools open directly.

`glot check` reports every pending entry as a warning, including `--format github` (pull-request annotations) and SARIF. Warnings do not fail the check.

## Translation memory

Every validated translation goes into `.glot/tm.json`: written translations that were not routed for review, `glot set`, queue approvals and edits. Each entry stores the source locale, target locale, source, target, and the key's note as context.

- **Exact match** (same locale pair, same source after whitespace normalization, same note): the stored translation is validated again, then written **without calling the LLM** (0 tokens, recorded as `origin: machine, model: "tm"`). `--force` skips this lookup because it asks for fresh output.
- **Fuzzy match** (normalized Levenshtein similarity of 85 % or more): the closest approved pair goes into the prompt as an example, so wording stays consistent across near-duplicates.

`glot translate` shows what the memory saved: `Translation memory: 120 strings reused (0 token; saves ~$0.0410)`.

To share a memory between projects, or with another TMS, use TMX 1.4:

```bash
glot tm export shared.tmx      # stdout without a file
glot tm import shared.tmx      # merges; multi-language <tu>s are split per target
```

Commit `.glot/tm.json` if you want CI runs to reuse the memory.

## Local models

```toml
[llm]
provider = "local"                     # Ollama, vLLM, LM Studio (OpenAI-compatible)
model = "translategemma:12b"           # or hunyuan-mt, qwen3, mistral-small3.2…
# base_url = "http://localhost:11434/v1"   # default (Ollama); vLLM: http://localhost:8000/v1
# api_key_env = "VLLM_API_KEY"             # only if your server wants a key

fallback_model = "claude-sonnet-5-5"   # escalation target
fallback_provider = "anthropic"        # another provider for the fallback
# fallback_base_url / fallback_api_key_env as needed
```

The local model does the first pass and its retries, which cost $0. A translation goes to `fallback_model` on `fallback_provider` when:

- the validator still rejects it after 2 retries (placeholders, plurals, tags, glossary), or
- the judge scores it under `qa.score_threshold`. The judge's notes go back to the model as issues to fix. The escalated value replaces the original only if it validates, and it is not judged again.

Recommended dedicated translation models:

| Model | Notes |
|---|---|
| **TranslateGemma** (4B / 12B / 27B) | Google's Gemma-based translation models (55 languages); 4B/12B run on a laptop. |
| **Hunyuan-MT** (7B, `Hunyuan-MT-Chimera` for ensembling) | Strong on Chinese and 33 languages; serve with vLLM. |
| General models (`qwen3`, `mistral-small3.2`, `gemma3`) | Better at keeping ICU/JSON output intact than small MT models. |

Small models more often break ICU plural syntax or return invalid JSON. The validator rejects these outputs, and escalation keeps the remote model as a safety net. To escalate valid but low-quality output too, set `judge_model` to the same value as `fallback_model`: the judge then runs on the fallback provider. Any other `judge_model` runs on the local provider.
