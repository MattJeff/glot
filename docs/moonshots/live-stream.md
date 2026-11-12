# Moonshot: live stream translation

## Problem
Every i18n tool, glot included, translates strings known in advance (catalogs, pages, CMS entries). Products now show
text that does not exist until it is generated or typed: AI answers, generated reports, notifications, chat. Today a
team either leaves it in one language, asks the LLM "answer in the user's language" (no glossary, no memory, no control,
and it does not apply to text produced by another system), or calls a translation API on the finished message (the
user waits for the whole answer).

## State of the art (checked 2026-11)
- **Simultaneous MT research**: wait-k / prefix-to-prefix (STACL, https://arxiv.org/pdf/1810.08398) emits k words behind
  the source and needs models trained for it; re-translation systems revise what was shown. Speech systems cut on
  sentence boundaries found by a splitter (https://arxiv.org/html/2508.07781v1). For text read on a screen revisions are
  visible flicker; for LLM output the "speaker" is fast and punctuated, so sentence-level waiting costs little.
- **Palabra realtime text API** (https://docs.palabra.ai/docs/streaming_api/realtime_text_translation): WebSocket, text
  fragments in, translations out, context settings. Built for ASR deltas: no Markdown/code handling, glossary or PII
  masking documented.
- **Lingo.dev SDK** `localizeText` / `localizeChat` (https://lingo.dev/en/sdk/javascript): on-demand translation of a
  finished message; progress callback, no streaming of a growing text.
- **Vercel AI SDK**: `experimental_transform` and language-model middleware can host a transform
  (https://ai-sdk.dev/docs/ai-sdk-core/middleware); transforms see chunks, so sentence buffering is left to the author.
  No translation middleware found.
- **Chat vendors** (Stream, PubNub: https://getstream.io/blog/real-time-chat-translation/): translate each finished message.
- **glot before this**: `ondemand.rs` translates a whole catalog for a new locale (a job, 202 then 200);
  `glot_llm::provider::Provider` has `complete()` only (no token streaming); the website tag patches DOM text against a
  precomputed memory and reports unknown segments for a later job. Nothing translated a stream.

**What is new**: a stream translator that is (1) Markdown/code aware with a proof that output does not depend on chunk
boundaries, (2) bound to a project's glossary, TM and style, (3) privacy-first (masking before the provider, hashes
only), (4) fail-open at every level, (5) a one-line wrapper around an LLM SDK stream. Sentence buffering itself is not new.

## Policy
- **Unit** = a sentence (`.!?…` + space + non-lowercase, or `。！？`), a table cell, or a line end; inline Markdown must
  be balanced at the cut; no cut found in 480 bytes = cut at a space. Decisions read the text up to one character past
  the cut, so they are identical however the stream was chunked (`markdown_and_code_survive_any_chunking`).
- **Revise-free**: emitted text is final. No `replace` event. Cost: lag = rest of the sentence + one provider call.
- **Raw passthrough**: fences (``` and ~~~), block markers, table pipes, JSON lines, whitespace, letterless text.
- **Protected spans** → `<xN/>` placeholders: inline code, link targets, URLs, HTML tags, do-not-translate terms, emails,
  digit runs ≥ 7. An answer missing/duplicating one, adding a line break or changing the count of `* \` | [ ]`, or 3×
  longer than the source is refused → source emitted (`invalid_output`).
- **Glossary**: matching terms in the prompt; `Glossary::check` on the result; one retry; still wrong = emitted, flagged.
- **Known text first**: conversation memory → project catalogs + glot Edge segments (exact match) → opt-in cache.
- **Fail-open**: provider error / 8 s timeout → source, flagged; 2 failures in a request or 1 in the last 20 s per
  project → provider skipped (no waiting on a dead provider). Client: any HTTP failure → rest of the stream as source.
- **Back-pressure / cancellation**: one request in flight per stream; inside it ≤ 3 batches ahead of the reader; the
  helper stops reading the source above 16 000 pending characters; dropping the response cancels provider calls.
- **Limits**: 32 kB per request, daily characters per project (default 500 000), `billing::check_llm` before each request.

## Architecture
```
LLM SDK stream ─► @glot/live-stream (translateStream)            crates/server/src/live_stream.rs
                   pending text ──POST {text, fence, mid, final}──►  auth, opt-in, cap, glossary, provider
                   ◄── SSE delta* , done{consumed, fence, mid} ───   glot_llm::live_stream::{Segmenter, run}
                   drops `consumed`, resends the tail + new text       Memory: conversation → TM → cache
```
**Transport: POST + SSE with a resumable cursor, not WebSocket.** Browsers cannot stream a request body while reading
the response (Chrome: half-duplex, HTTP/2 only; Safari/Firefox: no), so true duplex means WebSocket: a new dependency
(axum `ws`), a connection pinned to one replica holding the segmenter state, and trouble behind serverless/proxies. Here
the state is two fields the client carries (`fence`, `mid`) plus its own unconsumed tail, the server is stateless, any
`fetch` works, and one request is naturally one batch (what piled up while the previous one was translating). Cost: up
to one extra provider round-trip of lag versus a pipelined socket. To revisit with measurements.

Engine in `crates/llm/src/live_stream.rs` (no I/O besides the `Provider` and `Lookup` traits). The provider call uses
the existing `complete()` and the catalog payload shape, so every configured provider and the mock work unchanged.

## Data model (migration 0189)
`live_stream_settings` (enabled, browser, mask_pii, store, model, daily_chars), `live_stream_units` (keyed hash, hits,
target only with `store`), `live_stream_usage` (daily counters). Conversations: process memory, 30 min.

## Privacy / security
Opt-in per project (403 that says where to enable). Sent to the provider: masked units, matching glossary terms, last 4
masked source units. Not detected: names, postal addresses, IBAN letters. Stored: keyed hashes + counters; masked
translations only with `store`; purge endpoint. Public-key route is a separate switch, CORS-open like the Edge beacon,
bounded by the daily cap. Prompt injection: the system prompt declares fragments as data; structural checks + length
bound refuse answers that are not translations; a refused unit shows the source. No URL is fetched.

## Pricing idea
Meter characters (already counted per day). Included allowance per plan (e.g. Starter 1 M, Pro 10 M characters/month),
BYOK unlimited, memory/cache hits free: the bill falls as the product's own memory grows. Today: units translated on
glot's key count as `llm_strings`.

## Phases
1. **Built**: engine, HTTP API, TS helper, DOM helper, dashboard playground, docs, tests.
2. Token streaming inside a unit (`Provider::stream`) to show a long sentence as it is translated; `Provider` change.
3. Sentence-level TM: align multi-sentence catalog entries into sentence pairs (measured on ClipOnIt: 361 of 3677
   texts have several sentences, and their sentences miss the whole-string TM). Fuzzy TM hits as few-shot examples; per-language fast model routing; clause-level cuts for long sentences as an option.
4. AI SDK middleware package (`wrapStream`) and LangChain runnable; UI message stream parts (tool calls untouched).
5. Shared conversation store for multi-replica; WebSocket/WebTransport if measured lag justifies it.
6. PII: names via an optional local NER; IBAN; customer regexes.

## Risks
- Quality of sentence-by-sentence translation (pronouns, split clauses): context helps, not measured on a real model yet.
- Latency depends on the provider: one small call per batch; needs a fast model (setting `model`).
- Languages without sentence punctuation in the stream (Thai) fall back to line/480-byte cuts.
- Heuristics (abbreviations, unfenced multi-line JSON, tables without leading pipe) have documented limits.
- A model that reorders Markdown markers gets refused → more source text than expected; watch `failed_units`.

## Measured on the real projects (scrubbed copy `glot_real`, no AI call)
Replay of every short source text of each project (catalog + glot Edge, no ICU/HTML), 20 texts per message, each
message cut in the middle into two requests:
- ClipOnIt (fr → en/es/de/zh-CN, 16 glossary terms): 3677 texts, 139 346 characters, 4099 units. Served from the project's
  memory: 56.7 % (en, de, zh-CN), 73.0 % (es, + 2000 glot Edge segments). Everything else passed through flagged
  `no_provider`. Nothing lost (deltas cover every message exactly).
- Orizn (fr → nl, 12 do-not-translate terms, nl ~720/3868 translated): 3209 texts, 179 989 characters, 3923 units; 12.7 % from
  memory. 38 protected spans (links, emails, numbers) masked.
These are the projects' own UI texts, so the hit rate is an upper bound: real AI answers will reuse far fewer sentences.
