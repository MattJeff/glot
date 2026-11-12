# Two-way translation (slug `two-way`)

## Problem

glot translates what a company publishes. It ignores what customers write back. A French team that opens Japan with glot Edge gets Japanese tickets, form messages and emails the same week, and falls back to copy-pasting into a consumer translator: no glossary, no style guide, personal data pasted into a third-party tool, and no trace of what was actually sent.

## What exists (researched 2026-11)

| Tool | What it does | Where it stops |
| --- | --- | --- |
| Intercom AI Inbox Translation | Customer messages shown in the agent's language, replies converted back. Included with the Copilot add-on. Intercom's own help page warns that what the agent sees in the Inbox is not always what the customer received. | Only inside Intercom. No link to the product's glossary. https://www.intercom.com/help/en/articles/16296591-understanding-translation-behavior-in-intercom, https://www.intercom.com/changes/en/85031-support-your-customers-in-any-language-with-ai-inbox-translation |
| Zendesk native translation | Banner to translate a conversation when languages differ. | A community thread asks for a way to supply a glossary for incoming tickets. https://support.zendesk.com/hc/en-us/articles/4408832500506 |
| Language I/O, Lingpad, Unbabel, Translate.com | Dedicated add-ons for Zendesk/Freshdesk/Intercom/Salesforce: glossary, PII protection (vendor claims), human escalation for some. | A second glossary and a second vendor, separate from the site's localization; enterprise sales. Unbabel's glossary is a manually supplied spreadsheet/TBX. https://languageio.com/translation/zendesk/, https://www.lingpad.com/product/ai-translation/ticket-translation, https://www.eesel.ai/blog/support-translation-glossary |
| Freshdesk (Freddy AI) | Live ticket translation tied to Copilot licences. | Same silo. https://www.eesel.ai/blog/freshdesk-translation |

Not verified in this research: Crisp, Front and Help Scout built-in translation (no first-party source fetched).

What glot already had: the guarded translation pipeline (placeholder enforcement, glossary check, retries, fallback model), glossary and style guide per project, encrypted credentials, an SSRF-safe HTTP client, a transactional email outbox, Help Scout and Zendesk connectors for help-center **articles** only. Nothing touched conversations.

## What is new

Translating support messages is not new. What no tool above does:

1. **One glossary and one style guide for the site and for the conversations.** The term the customer reads on the product page is the term in the agent's reply, enforced by the same validator, in both directions, for any pair (the glossary is re-pivoted when neither side is the project's source language).
2. **The back-translation is the agent's proof, and the mode is explicit.** Every reply carries what the customer will read translated back, an agreement score, and is held when the score is low. `suggest` never sends; this is a server-side rule, not a UI convention.
3. **Tool-agnostic and open.** A signed webhook and a REST API, MIT-licensed, self-hostable: the same inbox for a helpdesk, a contact form and an email address.
4. **Customer languages as a market signal.** Counts of the languages customers write in, with the ones the project does not ship marked. It feeds the existing markets/launches features.
5. **PII masked by construction.** Placeholders are ICU arguments, so the existing validator refuses a translation that loses one.

## Architecture

`crates/server/src/two_way/`

- `detect.rs`: whatlang (local trigram + script model). Confidence scaled down for short Latin/Cyrillic/Arabic-script texts. Sentence-level vote for mixed languages.
- `pii.rs`: regex masker (symbols `{}<>`, URL, email, IBAN, Luhn-valid card, any number with 8+ digits, references with `#` or 5+ digits) → `{kind N}` placeholders, restored after; a lost placeholder is reported.
- `mod.rs`: `inbound` (detect → resolve language: confident detection > thread language > caller's hint > flagged guess → mask → translate → restore), `outbound` (mask → translate → back-translate the masked translation → agreement = Dice on character trigrams → `needs_review`), `pivot_glossary`, settings, storage, delivery, dashboard API, public channels (webhook, email, form, `form.js`).
- `helpscout.rs`: Help Scout Mailbox API 2.0 adapter (webhook HMAC-SHA1, OAuth2 client credentials with token cache, threads, notes, replies/drafts).

Translation goes through `glot_llm::translate::Translator` unchanged: same model, key, budget check (`billing::check_llm`), validators and retries as catalog jobs. The message is one item; the thread's last four messages are its `neighbors`; the instructions (faithful, do not soften, no length limit) are its `description`.

Known ceiling: the pipeline's system prompt is written for UI strings ("concise", "stay close to the source length"); the per-item description overrides it. A dedicated conversation prompt in `crates/llm` is the first thing to do with real traffic (phase 2).

## Data model (migration 0192)

- `two_way_settings` (one row per project): team locale, mode, thresholds, retention, webhook secret (AES-GCM), reply URL, form switches, forward address, Help Scout app id/secret/webhook secret (AES-GCM).
- `two_way_conversations`: channel, the other tool's thread id, customer locale, subject, `meta` (Help Scout customer id).
- `two_way_messages`: direction, original, translation, back-translation, confidence, flags, masked count, status (`received | draft | needs_review | sent | failed`), delivery, error.
- `two_way_lang_stats`: (project, day, channel, locale) → count. Survives retention purges; the only thing a contact form writes by default.

## Privacy and security

- To the model: masked text, masked thread context, glossary terms, style guide. Never: names supplied by the integrator, email addresses, ids.
- Not masked: names and postal addresses written inside the text (no NER). Stated in the docs; do not sell it as anonymization.
- Stored: message texts for the inbox, purged after `retention_days` (90). Contact form: nothing but the counter unless opted in. Inbound email: sender address used for the forward, conversation keyed by its SHA-256. Help Scout: numeric customer id only.
- Webhook: `Glot-Signature` (HMAC-SHA256, timestamp, 5 min tolerance; same code as the Stripe webhook). Help Scout: `X-HelpScout-Signature`. Secrets write-only, encrypted with `GLOT_SECRET_KEY`.
- Outbound HTTP (reply URL, Help Scout): `connectors::client_for` (http(s), public addresses, DNS pinned, no redirects).
- Public form endpoint: off by default, daily cap on translations, 5 000 characters max.
- Injection: customer text is data inside a JSON payload, and the output must pass placeholder and glossary validation; a message saying "ignore your instructions" can still bend a translation. The back-translation and the original stay visible to the agent; `auto` mode is the exposed one.

## Pricing idea

Metered like translation: a message counts as its strings do today (inbound = 1 call, outbound = 2). Starter: suggest mode, API + form. Growth: one-click and auto, helpdesk adapters, language-demand stats feeding Markets. BYOK stays unlimited. Not wired: there is no `two_way` meter in `billing.rs` yet, only the existing `check_llm` gate.

## Phases

1. **Built**: core, webhook + REST, contact form, inbound email (parse → translate → forward), Help Scout adapter, inbox, docs, tests.
2. Dedicated conversation prompt + judge-model score replacing the lexical agreement; billing meter; reply by email (needs a sending domain per project); Zendesk and Intercom adapters; realtime inbox (SSE).
3. Reviews/UGC (design only): the same `inbound` pipeline on public content. A review is stored with its original; the site shows "Translated from Japanese · see original" (the Edge proxy already rewrites pages, so the hook is `two_way::inbound` called from the Edge segmenter for nodes marked `data-glot-ugc`, cached by content hash like Edge segments). Differences with conversations: no PII restore in public output (masked items stay masked), no auto-publication of a flagged translation, and attribution of machine translation is mandatory. Not built: it needs an Edge change owned by another module.

## Measured on real data (scrubbed copy `glot_real`, local parts only, no LLM call)

`cargo test -p glot-server --test two_way real_data -- --ignored --nocapture`

- Detection, ClipOnIt (17 760 real strings, fr/en/es/de/zh-CN): when the detector says it is confident it is right 97–99 % of the time at every length; it is confident on 92 % of strings of 40–79 letters and on 100 % above 80, but only on 11–43 % under 40 letters. Short messages rely on the thread language and the hint by design.
- Detection, Orizn (825 usable strings, fr/nl): 97 % right when confident on 40–79 letters, 93 % above 80, after the Afrikaans→Dutch correction (before: 84 % and 79 %, 17 long Dutch strings read as Afrikaans).
- PII false positives on product copy (which contains no customer data): ClipOnIt 0.13 % of strings (URLs, one long reference), Orizn 1.45 % (the company's own email, a postal code, a CSP header). Over-masking is restored verbatim, so its cost is a slightly less informed model, not a wrong text.
- Glossary pivot, ClipOnIt: of the term occurrences found in real en/es/de strings, the French term glot would enforce is the one the real French copy uses 86 % (en), 97 % (es), 96 % (de) of the time; the rest are places where the human copy paraphrased ("une vidéo Pexels" for "a clip").
- Orizn: its 12 glossary terms are all do-not-translate brand names, and 3 763 of its 3 868 source strings are polluted by the mock provider (`⟦fr⟧ <English>`): not used as evidence.

## Risks

- **Wrong language on short messages.** Mitigated by thread language, hint, flag, manual correction; measured on real ClipOnIt strings (see the report).
- **False confidence.** The agreement score is lexical. It catches a reply that drifted, not a subtle politeness error in Japanese. `suggest` is the default for that reason.
- **Helpdesk APIs change.** One adapter, written against the public docs, tested on a fake; never run against a real Help Scout account yet.
- **Auto mode and prompt injection**: see above. Keep `auto` opt-in and behind the review threshold.
