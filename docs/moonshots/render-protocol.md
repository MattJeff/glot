# Render protocol — "the Stripe of language"

Status: first version built (server, dashboard, docs, two clients, tests). November 2026.

## Problem

A company translates its site with one tool and its app with another. Everything built on a server per recipient
is left out: transactional emails, invoices, push notifications, SMS, PDFs, exports. Those are written in a
backend, mixed with personal data, sent through a provider (Resend, SendGrid, Postmark, APNs) that knows nothing
about languages. The usual answer is one template per locale kept by hand, or nothing.

The protocol: any system hands glot **a piece of content and a recipient locale** and gets it back in that
language, with the project's glossary, memory, validation and tone. Instant when known, translated on the fly
when new, remembered after.

## What exists (market, checked November 2026)

- **DeepL API**: translates text and HTML (`tag_handling=html`), glossaries, translation memories. No template
  variables (tokens must be wrapped in tags by the caller), no per-request memory of your own content, a
  translation memory cannot be combined with a glossary or style in one call, no fallback contract, no locale
  formatting. https://developers.deepl.com/docs/xml-and-html-handling/html
  https://developers.deepl.com/api-reference/translate/request-translation.md
- **Lingo.dev**: the closest. A localization API (`/process/localize`) and SDK (`localizeText`,
  `localizeObject`, HTML), an "engine" holding model, glossary, brand voice; a guide for runtime email
  localization. The SDK "processes content on demand"; caching is documented for the CLI, not for runtime calls.
  No documented variable privacy, typed variable formatting, safe-fallback contract or idempotency.
  https://lingo.dev/en/guides/emails https://lingo.dev/en/docs/api/localize https://lingo.dev/en/sdk/javascript
- **Lokalise / Phrase / Crowdin**: TMS. They store keys and placeholders and sync templates (Crowdin has a Resend
  integration that creates one localized template copy per locale). Build-time, per template, not per request.
  https://store.crowdin.com/resend https://docs.lokalise.com/developer-docs/universal-placeholders
- **Email providers** (Resend, SendGrid, Postmark, ZeptoMail): the documented practice is one template per
  locale selected at send time, with strings in JSON files.
  https://www.zoho.com/zeptomail/articles/multi-language-transactional-emails.html
  https://dev.to/trevor_rawls_b670c3832b98/sending-multilingual-emails-with-postmark-and-gitloc-je5

### What glot already had

- `ondemand.rs`: on-demand **locales** for the catalog CDN (a whole locale is queued as a job, 202 then 200).
  Locale-level and asynchronous; nothing per piece of content.
- glot Edge (`glot_core::edge::segment`, `edge/tm.rs`): an HTML segmenter that masks values and PII, a
  hash-keyed translation memory, and a job that translates segments with the project's model, glossary and
  validators. Built for public web pages fetched by a proxy.
- `glot_core::markdown` (`glot docs`), `glot_core::validate`, `glot_core::plural`, the guarded `Translator`,
  `billing.rs` (plans, `consume`, LLM strings metering), MCP server, CDN.

### What is truly new

Honestly, not the idea of a translation API. What no listed product offers together, and glot did not have:

1. **Templates in, values never out.** Variables are placeholders in what is translated and stored; values are
   filled after, in memory. Numbers, URLs, emails and phones written in the content are masked too. Proven by a
   test that records what the provider receives.
2. **Unit-level deterministic memory.** A document is cut into paragraphs / segments, each remembered by hash.
   A changed paragraph costs one paragraph. Two emails that differ by an order number share everything.
3. **A fallback contract.** The answer is always sendable: translated, or the source with the reason. All or
   nothing per document by default (an email never mixes languages).
4. **One memory with the product.** A unit whose text is a catalog key's source, or an Edge segment, is served
   from the project's existing translations before any model call. The site, the app and the emails say the same
   thing.
5. **Typed variables formatted for the recipient** (CLDR numbers and dates, currency, percent, ICU plurals).

## Architecture

```
POST /v1/render ─► key (scope, quota) ─► plan: content → leaves → units (masked, hashed)
                                              │
                    ┌─────────────────────────┴───────────────────────────┐
                    ▼                                                     ▼
          render_tm (hash hit)                              miss: catalog (same source) → Edge TM
                    │                                                     │ still missing
                    │                                    Translator (project model, glossary, style)
                    │                                    + verify (placeholders, tags) → render_tm
                    └─────────────────────────┬───────────────────────────┘
                                              ▼
                 assemble (segmenter / Markdown parser) → integrity check (same tags, same links)
                                              ▼
                 any unit missing or broken → source document, status "fallback", reason
                                              ▼
                 variables applied (escaped in HTML, CLDR formatting) → response
```

- `crates/server/src/render_protocol.rs`: routes, key auth, quotas, idempotency, resolve, translate, usage, keys.
- `crates/server/src/render_protocol/content.rs`: content types → units and back; integrity checks. Reuses
  `glot_core::edge::segment` (HTML), `glot_core::edge::protect::mask` (values), `glot_core::markdown` (Markdown).
- `crates/server/src/render_protocol/vars.rs`: `{{mustache}}` fill, ICU MessageFormat formatter, number / date /
  currency / percent formatting (ICU4X `icu_decimal`, `icu_datetime`, same release train as glot-core).
- The model call runs in its own task: a caller that times out gets the source now and a cache hit next time.

## Data model (migration 0180)

| Table | Content |
|---|---|
| `render_keys` | Per-project keys: name, prefix, SHA-256 hash, scopes (`render`, `render:cached`), monthly limit. |
| `render_tm` | (project, locale, hash) → masked source, value, origin (`machine` / `catalog` / `edge`), model, hits. |
| `render_usage` | Counters per (project, day, key): documents, units, cached, translated, fallbacks, cost. |
| `render_idempotency` | (project, key) → request hash, localized templates before variables. 24 h. |

## Privacy and security

- Sent to the LLM: masked unit texts and the caller's optional `context`. Never: variable values or names,
  numbers, URLs, emails, phone numbers found in the content.
- Stored: masked units and their translations; counters. Never: variables, rendered output, request bodies.
- Not masked: a name typed directly into the content instead of a variable. The docs say so.
- Keys: hash only, shown once, revocable, scoped; `render:cached` cannot spend.
- No server-side fetch of customer URLs: no SSRF surface. HTML is never executed; the dashboard previews it in
  an `<iframe sandbox>` without scripts.
- Variables in HTML are escaped unless written `{{{raw}}}`.

## Pricing idea

Meter: documents rendered per month (`billing::RENDER`, quota per plan: Free 1 000, Starter 20 000, Pro 200 000,
Team 2 000 000 — placeholders to tune), cache hits included because that is the value delivered. New
translations with glot's key count in the existing `llm_strings` quota and its metered overage; BYOK is never
limited. Later: a usage-based price per 1 000 documents above the plan, on the existing Stripe meter mechanism.

## Phases

1. **Built**: endpoint + batch, six content types, memory, catalog and Edge reuse, fallback, idempotency, keys,
   scopes, quotas, usage, playground, docs with recipes, TypeScript and Python clients.
2. Review and edit of the render memory in the dashboard; purge on glossary change; context in the hash for
   ambiguous short texts; a prose-oriented system prompt (today's is written for UI strings).
3. OpenAPI and MCP tool (`render`); background warm-up of a template in every target locale; webhooks when a
   fallback is later translated; IANA time zones; ICU4X currency formatter when stable.
4. Provider plugins that sync localized templates (SendGrid, Postmark) from one source template.

## Risks

- **Callers send finished text instead of templates** ("Hello Ada"): every send is a miss and a cost. Mitigation:
  number / URL / email masking, the hit rate in the dashboard, key limits, docs. Names cannot be detected safely.
- **Latency on a miss** (seconds). Mitigation: `timeout_ms`, `cache_only`, `render:cached` keys, warm-up.
- **Quality of long prose** with a prompt tuned for UI strings: phase 2.
- **A wrong translation is remembered**: until the memory is editable, glossary / catalog are the levers.
- **Duplicate spend** on two identical first requests at the same instant (documented `ponytail:` ceiling).
