# Render API (emails, notifications, documents)

Your site and your app are translated. Your emails, invoices, push notifications and PDFs usually are not: they are built on a server, per recipient, far from any i18n library. The render API is one endpoint for all of them. You send a piece of content and the recipient's locale. You get the same content back in that language, with the project's glossary, style, translation memory and validators applied.

- **Known content is instant.** Each paragraph or HTML segment is remembered by hash. The same template never calls a model twice, whatever the variables.
- **New content is translated on the fly**, checked, and remembered.
- **You never get broken content.** If a translation is missing or fails a check, the whole document comes back in the source language with `status: "fallback"` and the reason.
- **Variables stay private.** Only templates are translated. Variable values are filled in afterwards, in memory. They are never sent to a model, never stored, never logged.

## Quick start

Create a key in the dashboard (Project → Render API → Keys). Then:

```sh
curl https://glot.example.com/v1/render \
  -H "Authorization: Bearer glot_rk_…" \
  -H "Content-Type: application/json" \
  -d '{
    "locale": "fr-FR",
    "type": "email",
    "content": {
      "subject": "{{first_name}}, your order has shipped",
      "html": "<h1>Hi {{first_name}}</h1><p>Total paid: {{total}}. <a href=\"{{tracking_url}}\">Track my parcel</a></p>"
    },
    "variables": {
      "first_name": "Zoë",
      "total": { "type": "currency", "value": 1234.5, "currency": "EUR" },
      "tracking_url": "https://track.example.com/p/9f3a"
    }
  }'
```

The answer has the same shape as `content`:

```json
{
  "locale": "fr-FR",
  "type": "email",
  "status": "translated",
  "content": { "subject": "…", "html": "…" },
  "cache": "miss",
  "units": { "total": 3, "cached": 0, "translated": 3, "fallback": 0 },
  "warnings": []
}
```

Send the same request again and `cache` is `hit`: no model call, a few milliseconds.

## Request

| Field | Meaning |
|---|---|
| `locale` | The recipient's locale (BCP 47: `fr`, `pt-BR`; `fr_FR` is accepted). It does not need to be one of the project's target locales. The project's source locale returns the content as is, with variables filled. |
| `type` | `text` (default), `html`, `markdown`, `template`, `email` or `json`. |
| `content` | A string, or an object for `email` and `json`. |
| `variables` | Optional. Values for the placeholders. Without it you get the localized template back and fill it yourself. |
| `fields` | `json` only. Which paths are text. |
| `context` | Optional. What this content is ("password reset email", "push notification, max 60 characters"). It guides new translations. Do not put personal data in it: it is sent to the model. |
| `allow_partial` | Optional. Accept a document whose untranslatable parts stay in the source language. Default: all or nothing. |
| `cache_only` | Optional. Never call a model for this request. Unknown content returns in the source language at once. |
| `timeout_ms` | Optional. How long to wait for new translations (default 20000). After that you get the source back, the translation continues in the background, and the next identical request is served from memory. |

### Content types

| Type | `content` | What is translated | What is kept exactly |
|---|---|---|---|
| `text` | string | Each paragraph (blank-line separated) | Line breaks, indentation, `{{tokens}}`, numbers, URLs, emails |
| `html` | string | Text of each block, and `alt`, `title`, `aria-label`, `placeholder`, meta descriptions | Every tag and attribute, links, CSS, scripts, comments (including Outlook conditional comments), `{{tokens}}` |
| `markdown` | string | Paragraphs, headings, list items, table cells | Code, link targets, HTML, front matter keys |
| `template` | string | One ICU MessageFormat message | Arguments, `plural` / `select` structure (plural categories are adapted to the target language) |
| `email` | `{subject, html, text}` (any subset) | Subject and text as `text`, `html` as `html` | The email is all or nothing: never a French subject over an English body |
| `json` | any JSON | Only the strings matched by `fields` | Everything else: ids, numbers, enum values, structure |

`fields` is a list of dotted paths. `*` matches every item of an array or object. A field is plain text unless you give it a type:

```json
{
  "locale": "de",
  "type": "json",
  "fields": ["title", "lines.*.label", { "path": "note", "type": "markdown" }],
  "content": {
    "id": "inv_42",
    "title": "Invoice",
    "lines": [{ "label": "Yearly plan", "sku": "PLAN-Y", "amount": 1200 }],
    "note": "Paid by **card**."
  }
}
```

### Variables

Two syntaxes, depending on the type:

- **`{{name}}`** in `text`, `html`, `markdown`, `email` and JSON fields. In HTML the value is escaped; `{{{name}}}` inserts it as is (for URLs you built yourself). `{{user.city}}` reads a nested object. A name you do not provide is left in place, so your email provider can still fill it. Block tags (`{{#if}}`, `{% for %}`) are left untouched.
- **ICU MessageFormat** in `template`: `{name}`, `{count, plural, one {# file} other {# files}}`, `{gender, select, …}`, `{n, number}`, `{d, date, long}`. Every argument must be provided, or the request is refused before anything is translated.

A plain string or number is inserted as written (an order number is not a quantity). To format a value for the recipient's locale, type it:

| Variable | `en-US` | `fr-FR` | `de-DE` |
|---|---|---|---|
| `{"type": "currency", "value": 1234.5, "currency": "EUR"}` | €1,234.50 | 1 234,50 € | 1.234,50 € |
| `{"type": "number", "value": 1234567.891}` | 1,234,567.891 | 1 234 567,891 | 1.234.567,891 |
| `{"type": "percent", "value": 0.256}` | 26% | 26 % | 26 % |
| `{"type": "date", "value": "2026-11-11", "style": "long"}` | November 11, 2026 | 11 novembre 2026 | 11. November 2026 |
| `{"type": "datetime", "value": "2026-11-11T09:30:00+01:00"}` | Nov 11, 2026, 9:30 AM | 11 nov. 2026, 09:30 | 11.11.2026, 09:30 |

Options: `min_fraction` / `max_fraction` (number), `max_fraction` (percent), `style` = `short` / `medium` / `long` (dates). A `datetime` is shown at the UTC offset written in the value: convert it to the recipient's offset before calling. Numbers and dates use CLDR data. Currency and percent patterns cover the common languages; an unknown language gets the English pattern.

## Answer

| Field | Meaning |
|---|---|
| `status` | `translated`, `source` (the locale is the project's source locale), `partial` (only with `allow_partial`), or `fallback`. |
| `content` | Always safe to send. On `fallback` it is your source content, variables filled. |
| `fallback_reason` | Why, starting with a code: `provider_error:`, `validation:`, `timeout:`, `not_cached:`, `llm_unavailable:`, `markup_integrity:`. |
| `cache` | `hit` (nothing new), `partial`, `miss`, or `none` (nothing to translate). Also in the `X-Glot-Cache` header. |
| `units` | Translatable units in the document: `total`, `cached`, `translated` (new, paid once), `fallback`. |
| `warnings` | For example a `fields` path that matched nothing. |

Errors use the usual envelope, `{"error": {"code", "message"}}`:

| Status | Code | Meaning |
|---|---|---|
| 400 | `validation_error` | The request is wrong; the message says what to change (unknown type, invalid ICU template, missing variable, bad date). Nothing was spent. |
| 401 | `unauthorized` | Missing, unknown or revoked render key. |
| 402 | `quota_exceeded` | The plan's monthly documents are used. |
| 413 | `too_large` | More than 512 KB. Split the request. |
| 422 | `idempotency_key_reused` | The same `Idempotency-Key` with a different body. |
| 429 | `key_quota_exceeded` | This key's own monthly limit is reached. |

A provider failure is **not** an HTTP error: you get `200` with `status: "fallback"`, because you still have an email to send.

## Batch

`POST /v1/render/batch` takes up to 50 items and answers `{"items": [...]}` in the same order. Use it for the same content in several languages, or several contents at once. Each item succeeds or carries its own `error`. Units shared between items are translated once.

```json
{ "items": [
  { "id": "fr", "locale": "fr", "type": "text", "content": "Your export is ready." },
  { "id": "ja", "locale": "ja", "type": "text", "content": "Your export is ready." }
] }
```

## Idempotency

Send an `Idempotency-Key` header (any string up to 255 characters) with a request you may retry. For 24 hours, the same key with the same body returns the first answer, with the header `Idempotent-Replayed: true`. A replay is not counted in your quota. Only complete answers are stored: a `fallback` retried with the same key gets a new chance. What is stored is the localized template, not your variables.

## Keys, scopes and quotas

Keys belong to one project and start with `glot_rk_`. glot stores only their hash; the key is shown once.

| Scope | Can do |
|---|---|
| `render` | Serve known content and translate new content with the project's model. |
| `render:cached` | Serve known content only. It never calls a model, so it can never spend. Unknown content returns `fallback` with `not_cached`. |

Each key can have its own monthly limit. On glot cloud, plans include a number of documents per month (cache hits count as documents; new translations made with glot's LLM key also count as translated strings, as in jobs; with your own LLM key they are never limited). Self-hosted servers have no limit.

## What is sent where

| Data | Your request to glot | glot's database | The LLM provider |
|---|---|---|---|
| Templates (text with `{{tokens}}`) | yes | yes, masked, as translation memory | yes, masked, only when new |
| Variable values and names | yes, in memory for the request | no | no |
| Numbers, URLs, email addresses, phone numbers written in the content | yes | no (they are `{0}` placeholders in a unit) | no |
| `context` | yes | no | yes |

A unit is the masked text: `Your order {0} left our warehouse` or `Hi {v0},`. This is also why `Your order #48213 …` and `Your order #48214 …` share one translation. A person's name written directly in the content (not as a variable) is part of the text and is sent: use a variable.

## Clients

TypeScript (`packages/render`, zero dependencies):

```ts
import { createClient } from "@glot/render";

const glot = createClient({ key: process.env.GLOT_RENDER_KEY!, baseUrl: "https://glot.example.com" });
const mail = await glot.render({ locale: user.locale, type: "email", content: { subject, html }, variables });
```

Python (`packages/render-python/glot_render.py`, one file, standard library):

```python
from glot_render import Glot

glot = Glot(key=os.environ["GLOT_RENDER_KEY"], base_url="https://glot.example.com")
r = glot.render(locale=user.locale, type="text", content="Your order {{id}} has shipped", variables={"id": order.id})
```

Both fail open: if glot cannot be reached, they return the source content with variables filled and `status: "fallback"`. A refusal you must fix (400, 401, 402, 429) is raised.

## Recipes

### Resend

```ts
const mail = await glot.render({
  locale: user.locale,
  type: "email",
  context: "Welcome email of a video editing app",
  content: { subject: "Welcome, {{first_name}}", html: await render(<WelcomeEmail />), text: plainText },
  variables: { first_name: user.firstName },
}, { idempotencyKey: `welcome-${user.id}` });

await resend.emails.send({ from, to: user.email, subject: mail.content.subject!, html: mail.content.html!, text: mail.content.text });
```

Render your React Email component with `{{first_name}}` as literal text, not with the real name: that is what keeps the template cacheable and the name private.

### SendGrid dynamic templates, Postmark templates

These providers store the template and fill `{{handlebars}}` themselves. Localize the template once per language, without `variables`, and keep one template per locale:

```ts
for (const locale of ["fr", "de", "ja"]) {
  const t = await glot.render({ locale, type: "email", content: { subject: tpl.subject, html: tpl.html_content } });
  if (t.status === "translated") await sendgrid.createTemplateVersion(templateIdFor(locale), t.content);
}
```

Tokens, `{{#if}}` blocks and `{{{unsubscribe}}}` come back untouched. With Postmark, do the same with `HtmlBody`, `TextBody` and `Subject` and one template alias per locale (`welcome-fr`). Or skip stored templates and render at send time, as with Resend.

### Push notifications and SMS

Use `template` for counts and `text` otherwise. Say the limit in `context`.

```ts
const n = await glot.render({
  locale: device.locale,
  type: "template",
  context: "Push notification title, max 50 characters",
  content: "{count, plural, one {# new clip is ready} other {# new clips are ready}}",
  variables: { count: 3 },
});
await apns.send(device.token, { alert: n.content });
```

For latency-critical sends use a `render:cached` key or `cache_only: true`, and warm the memory once at deploy time with a `render` key.

### PDFs and invoices

Localize the data, then feed your existing PDF template.

```ts
const doc = await glot.render({
  locale: customer.locale,
  type: "json",
  fields: ["title", "lines.*.label", "footer"],
  content: invoice,
});
const total = await glot.render({
  locale: customer.locale,
  type: "template",
  content: "{total}",
  variables: { total: { type: "currency", value: invoice.total, currency: invoice.currency } },
});
const pdf = await renderPdf(doc.content, { total: total.content });
```

Legal mentions that must stay in the language of the seller: leave them out of `fields`.

### Server-rendered pages

For whole sites, [glot Edge](/docs/edge) translates every page with no code. Use the render API for the fragments built per request that never exist as a page: an account banner, a generated summary, an export.

```ts
const banner = await glot.render({ locale: req.locale, type: "html", content: bannerHtml, cache_only: true });
res.send(layout(banner.content));
```

## How it relates to the rest of glot

- It reuses what the project already knows: a catalog key whose source is exactly a unit's text, and glot Edge's translation memory, are served before any model call.
- New translations use the project's model, glossary and style (Project → Settings), through the same validators as translation jobs.
- The memory is separate from the catalog: rendering never creates keys.

## Limits of this version

- A translation in the render memory cannot be edited from the dashboard yet. To impose a wording, add a glossary term, or a catalog key with the same source text, before the first render.
- The context of a unit is not part of its hash: the first translation of a text is reused for every later use in the project.
- No IANA time zones: a `datetime` is shown at the offset in the value.
- Not in the OpenAPI document or the MCP server yet.
