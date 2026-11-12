# Live stream translation

Catalogs translate text you know in advance. An AI answer, a generated report, a notification or a chat message does not exist until the moment it is produced. Live stream translation takes that text **while it streams** and gives it back in another language, with the project's glossary, translation memory and style guide. It requires the [server](self-hosting.md) (or glot cloud) and an AI provider key on the project.

## Turn it on

Dashboard → **Live stream** → switch it on. It is off by default because the text is your users' content: the page states what is sent to your AI provider and what is stored (see [Privacy](#privacy)). The same page has a playground: type or paste a message, it is replayed through the real pipeline and the latency is measured in your browser.

## One line in a route handler

```ts
import { translateStream } from "@glot-i18n/live-stream";

export async function POST(req: Request) {
  const result = streamText({ model, messages });          // any LLM SDK: a ReadableStream or an async iterable of text
  return new Response(
    translateStream(result.textStream, {
      project: "acme",
      to: "es",                                             // the reader's language
      token: process.env.GLOT_TOKEN,                        // API token (dashboard → API tokens); server side only
      conversation: chatId,                                 // optional: keeps wording identical across turns
    }).pipeThrough(new TextEncoderStream()),
  );
}
```

`translateIterable(source, opts)` is the same thing as an async generator. Both accept text or bytes, work in Node, edge runtimes and browsers (plain `fetch`), and take `signal` to cancel, `from` to name the source language (detected otherwise), `endpoint` for a self-hosted server and `onEvent` to observe every piece.

## What your user sees

- **Final text only.** Text is held back until a sentence (or a line) is complete, then shown translated. What was shown never changes: there is no flicker and no "replace" step. The lag is one sentence plus one call to your AI provider.
- **Valid Markdown at every moment.** Headings, lists, quotes and tables keep their markers. Fenced code blocks, inline code, URLs, link targets, HTML tags, JSON lines and numbers come out byte for byte.
- **Your terms.** Glossary translations are required from the model and checked; a miss is retried once. Do-not-translate terms never reach the model at all. A sentence that exists in your catalogs or glot Edge memory is served from there, without any AI call.
- **Consistency.** With a `conversation` id, a sentence already translated in the conversation comes back identical, and new sentences are translated knowing the previous ones.

## When something fails

The stream never breaks and nothing is lost: the affected text passes through in the source language.

| What happened | What the reader gets | How you know |
|---|---|---|
| The AI provider errors or takes more than 8 s | the source sentence | `delta` with `via: "source"`, `reason: "provider_error"` / `"provider_timeout"`; the provider is skipped for 20 s (`"provider_down"`) |
| The model lost a protected element or broke the Markdown | the source sentence | `reason: "invalid_output"` |
| No AI key on the project | memory hits translated, the rest as source | `reason: "no_provider"` |
| glot unreachable, feature off, daily cap reached | the rest of the message as source | `onEvent({ type: "fallback", reason })` |

```ts
translateStream(source, { ...opts, onEvent: (e) => e.type === "fallback" && log.warn("glot live stream", e.reason) });
```

## In the browser

For text rendered by a page you do not control the stream of (a widget, a site builder), load the script and mark the container. Text nodes that grow inside it are shown translated; the source is never painted. `<pre>`, `<code>`, `[translate="no"]` and `.notranslate` are left alone.

```html
<script src="https://glotlang.com/widget/glot-live.js" data-project="acme" data-key="pk_…" defer></script>
<div data-glot-live="es">…</div>
```

This uses the project's public key and needs **Browser access** switched on. With a bundler: `import { observeLive } from "@glot-i18n/live-stream/dom"`. Prefer `translateStream` on your server when you own the stream: it sees whole sentences, the DOM helper sees text nodes.

## Privacy

- **Sent to your AI provider:** the sentences to translate, the glossary terms they contain, the last four sentences of the conversation. Before that, code, links, HTML tags and do-not-translate terms are replaced by placeholders, and so are email addresses and numbers of 7 digits or more (phones, card and account numbers) unless you turn masking off. They are put back in the translation; a translation that lost one is refused. **Names and postal addresses are not detected.**
- **Stored by glot:** per sentence, a hash keyed with the server secret and a counter; per day, totals (requests, characters, sentences, failures). No text.
- **Cache (opt-in):** keeps the masked translation of each sentence so repeats are instant and free. Turning it off deletes them; `DELETE /api/projects/{slug}/live-stream/cache` forgets everything.
- **In memory:** a conversation's masked sentences, 30 minutes, never written to disk.

## HTTP API

`POST /api/projects/{slug}/live-stream/translate` (`Authorization: Bearer <token>`) or `POST /live/v1/{slug}/translate` (public key in the body, browsers).

```json
{ "to": "es", "from": "en", "text": "…", "final": false, "conversation": "c1", "fence": null, "mid": false }
```

The answer is Server-Sent Events: `delta` events `{"text", "n", "via", "reason"?}` in order, then `done` `{"consumed", "fence", "mid", "stats"}`. The server keeps no stream state: send the text you have; it translates the stable part and tells you how many UTF-16 units it consumed; send the rest again with what arrived since, passing `fence` and `mid` back; set `final` with the last text. Closing the response cancels the work in flight. Limits: 32 kB of text per request, a daily character cap per project (500 000 by default).
