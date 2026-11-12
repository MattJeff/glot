# Two-way translation (customer conversations)

Your site speaks your customers' languages. This makes the conversation work in both directions: a customer writes in Japanese (ticket, contact form, chat, email), your team reads it in French next to the original, answers in French, and the customer receives natural Japanese, written with the same glossary and style guide as your site. It requires the [self-hosted server](self-hosting.md) or glot cloud.

Everything lands in **Conversations** in the dashboard: original and translation side by side, a reply box that shows what the customer will read translated back for you, and the languages your customers actually write in.

## What happens to a message

1. **Language detection, on the server.** No network call, no cost. A short message ("ok merci") cannot be detected reliably: glot then uses the language the customer used earlier in the thread, then the `lang_hint` you sent, and only then a guess, which it flags as *Language unsure* so a person can set it.
2. **Personal data is hidden before the model sees anything.** Email addresses, URLs, phone numbers, IBANs, card numbers and order or reference numbers are replaced by placeholders (`{email1}`, `{ref1}`…) and put back after translation. The pipeline refuses a translation that loses a placeholder. Names and postal addresses written in free text are *not* detected.
3. **Translation with your glossary and style guide**, whatever the pair. A term known in English, French and Japanese is enforced from Japanese to French and back, even when your project's source language is English. The previous messages of the thread are given as context.
4. **The customer's tone is not edited.** Urgency, emphasis and rudeness are translated as they are; glot does not summarize or classify.
5. **Your reply is translated, then translated back.** You read the back-translation before anything leaves. When it differs too much from what you wrote, the reply is flagged *Needs review*.

The original is always stored next to its translation.

## Who sends: three modes

| Mode | What glot does with your reply |
| --- | --- |
| **Suggest** (default) | Translates it. Never sends. You copy it, or find it as a draft in your helpdesk. |
| **One-click send** | Sends when a person presses **Send** on that reply. |
| **Automatic** | Sends as soon as it is translated, except replies flagged *Needs review*, which wait for a person. |

## Connect any tool: REST or signed webhook

With an [API token](self-hosting.md), from your backend:

```bash
curl -X POST https://glot.example.com/api/projects/acme/two-way/inbound \
  -H "Authorization: Bearer $GLOT_TOKEN" -H "Content-Type: application/json" \
  -d '{"conversation_id": "ticket-42", "message_id": "m-1", "text": "注文した商品がまだ届いていません。", "lang_hint": "ja"}'
```

The response is the stored message: `original`, `original_locale`, `translation`, `translation_locale`, `flags`, `pii_masked`. `conversation_id` and `message_id` are your tool's own ids (any string); sending the same `message_id` twice returns the first result. Only `conversation_id` and `text` are required.

Without a token (a helpdesk automation, Zapier, a worker), use the webhook: generate a secret in **Conversations → Settings** and sign each request.

```
POST /api/two-way/{project}/webhook
Glot-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256(secret, "<t>.<raw body>")>
```

```js
const t = Math.floor(Date.now() / 1000);
const v1 = crypto.createHmac("sha256", secret).update(`${t}.${body}`).digest("hex");
await fetch(url, { method: "POST", headers: { "Glot-Signature": `t=${t},v1=${v1}`, "Content-Type": "application/json" }, body });
```

Requests older than five minutes or with a wrong signature get `401 bad_signature`.

To answer from your tool:

```bash
# translation + back-translation, nothing stored
POST /api/projects/acme/two-way/preview            {"conversation_id": "<glot id>", "text": "Bonjour, …"}
# stored as a draft (or sent, depending on the mode)
POST /api/projects/acme/two-way/conversations/<glot id>/reply   {"text": "Bonjour, …", "send": false}
POST /api/projects/acme/two-way/messages/<message id>/send
```

A sent reply is posted to your **Reply URL** when you set one (same `Glot-Signature`, body `{event: "reply", conversation_id, message_id, text, locale, original}`); otherwise read it from the API response and deliver it yourself. In Suggest mode, `send` answers `409 mode_forbids_send`.

## Contact form

Turn on **Contact-form helper** in Settings, then add one line to the page and one attribute to the form:

```html
<script src="https://glot.example.com/api/two-way/form.js" data-project="acme" async></script>
<form data-glot-form action="https://your-form-backend.example" method="post">
  <textarea name="message"></textarea>
  <button>Send</button>
</form>
```

On submit, the script adds two hidden fields, `glot_language` and `glot_translation`, then lets the form go where it was going. Your form backend (Formspree, Netlify Forms, your own) receives the message, its language and its translation in your team's language. If glot does not answer within 8 seconds the form is submitted without them: the script never blocks a submission.

By default **nothing is stored**: glot keeps a count per language and per day, not the message. Turn on *Also keep form messages in this inbox* to store them. The endpoint is public, so translations are capped per day (200 by default); above the cap the language is still returned, without a translation.

Forms submitted by JavaScript (`fetch`) do not go through a native submit: call `POST /api/two-way/{project}/form` with `{"text": "…"}` yourself and use the answer.

## Help Scout

Without leaving Help Scout:

- a customer writes → glot adds a **note** with the translation in your language;
- you answer with a note that starts with `/glot ` in your language → glot writes the reply in the customer's language: a **draft reply** in Suggest mode, a sent reply in Automatic mode, and a note with the back-translation either way.

Setup: in Help Scout, create an app (**Your Profile → My Apps**) and copy its App ID and App Secret into **Conversations → Settings**. Then create a webhook (**Manage → Apps → Webhooks**) pointing to `https://glot.example.com/api/two-way/{project}/helpscout`, with a secret key of your choice (paste the same one in glot) and the events *Conversation created*, *Customer replied* and *Note created*. Credentials are encrypted at rest. glot keeps the Help Scout customer id (needed to reply), not the customer's name or address.

## Inbound email

`POST /api/two-way/{project}/email`, signed like the webhook, with `{"from", "subject", "text", "message_id"}` (what an inbound-mail provider or an email worker gives you). glot drops the quoted history, translates the new part and forwards it to the address set in **Forward translated emails to**, original below. The sender's address is used for the forward only; the conversation is keyed by a hash of it. Replying by email from glot is not available yet.

## Which languages do customers write in?

**Conversations** shows the count of real customer messages per language over the last 90 days, and marks the languages your project does not ship. Customers already writing in a language you do not serve is a strong, free signal for the next market to open. `GET /api/projects/{project}/two-way/stats?days=90` returns the same numbers.

## Privacy

- Sent to the model: the message text with the personal data listed above replaced by placeholders, the last four messages of the thread (masked the same way), your glossary terms and style guide. Never: the customer's name, email address or any id.
- Stored: message texts (original and translation) for the inbox, deleted after the retention period (90 days by default); contact-form messages only when you opt in.
- Pattern-based masking has limits: a name or a street address inside the text is not hidden. Do not treat it as anonymization.

## Limits

- Glossary terms in languages written without spaces (Japanese, Chinese) are matched as plain substrings.
- The confidence of a reply is a lexical comparison between what you wrote and its back-translation: a faithful paraphrase can score low. Lower the review threshold in Settings if it flags too much.
- One helpdesk adapter today (Help Scout). Any other tool connects through the webhook or the REST API.
