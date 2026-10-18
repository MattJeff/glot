# Live review

Let your team and outside reviewers annotate translations directly on your real site: pin a comment on a text, discuss it in a thread, ask the AI for a shorter or clearer wording, preview it in the page and apply it. Everything lands in the dashboard (**Review**), the CLI and the MCP server. It requires the [self-hosted server](self-hosting.md) or glot cloud.

![A reviewer pins a comment on an overflowing German button, asks for a shorter wording and applies it](/launch/review.gif)

## Install in one line

Add the loader to your layout, once:

```html
<script src="https://unpkg.com/@glot-i18n/review/dist/review.global.js"
        data-project="acme" data-endpoint="https://glot.example.com" async></script>
```

or, with a bundler:

```ts
import { init } from "@glot-i18n/review";

init({ project: "acme", endpoint: "https://glot.example.com", locale: "de" });
```

Then, in the dashboard (**Review → Settings**), declare the origins the widget runs on (`https://www.acme.com`, `http://localhost:3000`). The server refuses widget requests from any other origin.

The loader weighs about 1 kB and does nothing for normal visitors: no network request, no cookie, no DOM change. The widget itself is lazy-loaded only when the tab holds a review token.

> **Note:** a [working example](https://github.com/MattJeff/glot/tree/main/examples/review-demo) (Next.js, en/fr/de) runs the whole flow end to end, with its Playwright tests.

## Review mode

Open the site with a review link (below) or from **Review** in the dashboard. A small toolbar appears (movable, keyboard accessible):

- **Comment**: click any text — or Tab between texts and press Enter — to drop a pin. The widget detects the glot key (`data-glot-key`, the runtime dev markers, or the server matching the rendered text against the catalog); a text that matches no key is still accepted.
- **Thread**: reply, mention a teammate with `@`, react, edit or delete your own messages. Bodies accept `**bold**`, `*italic*`, `` `code` `` and links.
- **Status**: open → in progress → resolved / won't fix (members).
- **Realtime**: pins, replies and status changes appear in every open tab and in the dashboard within a second (Server-Sent Events).
- **Re-anchoring**: pins follow their text when the page changes — by key first, then by CSS selector, then by searching the text. A pin whose text is gone is shown as *orphan* in the list instead of floating at a wrong place.

In the dashboard, **Review** shows your site in a frame with the same pins and the full thread on the side. If your site forbids being framed (`X-Frame-Options`, `frame-ancestors`), the page falls back to the screenshot captured with each annotation; use **Open the site** to review it live.

## Review links

Share a link with translators or clients who have no glot account: **Review → Links → New link**.

| Access | Can |
|---|---|
| `read` | see pins, threads and screenshots |
| `comment` | + create annotations, replies, reactions; edit or delete their own |
| `edit` | + edit texts, apply suggestions, change statuses |

The link is `https://www.acme.com/?glot_review=glr_…`. The widget moves the token to `sessionStorage` (this tab only) and removes it from the address bar. Reviewers type their name once; it signs their messages. Links expire after 14 days by default (1–365 days, or never) and can be revoked at any time — revocation is immediate.

Members do not need a link: the dashboard opens the site with a 12-hour member session.

## AI rewording

In an annotation, **Reword** proposes a new text for the key in the displayed language: *Shorter*, *Clearer*, *More formal*, *Warmer*, or *Fit the segment* (the width measured on the page). The proposal is checked by the validator (placeholders, ICU plurals, glossary, pixel budget) before you see it, and **Preview on the page** swaps the text in place so you can check the layout.

**Apply** writes it as the reviewed translation of that key (origin *human*, author = you, kept in the key's history) and resolves the annotation. An invalid text is refused (`422 invalid_translation`) and nothing is written. You can also type your own text; it goes through the same checks. Reviewers on a `comment` link can propose a text; a member applies it.

## CLI and MCP

Agents and scripts see the same annotations:

```sh
glot review list --status open           # open annotations: id, route, locale, key, body
glot review show <id>                    # thread, suggestion, current translation
glot review reply <id> "Fixed in #42"
glot review resolve <id>
glot review apply <id> [--value "…"]     # write the suggestion as the reviewed translation
```

The MCP server exposes the same operations as tools (`review_list`, `review_get`, `review_reply`, `review_resolve`, `review_apply`), so an agent can read what reviewers asked, fix the text or the layout, and close the loop. Over HTTP, the routes live under `/api/projects/<slug>/review/…` with an API token (`Authorization: Bearer glot_…`).

## Privacy

- **Normal visitors**: nothing is collected. Without a review token the loader returns immediately: no request to glot, no cookie, no storage, no fingerprint.
- **Reviewers**: what they write (comments, name typed for a link, reactions), the annotated element (CSS selector, text, page URL without query string or fragment, viewport size) and, optionally, a cropped screenshot of the element. No IP address or user agent is stored with an annotation.
- Screenshots are stored in your glot database, served only to people with access to the project, and deleted with the annotation.
- The widget runs in a Shadow DOM and never reads form fields.

## FAQ

**Does it slow down my site?** No. Normal visitors only parse a ~1 kB script that checks for a token and exits. Load it `async`.

**Do I need to tag my texts?** No. The server matches the rendered text against your catalogs. `data-glot-key` attributes or the `@glot-i18n/runtime` dev markers make the match exact when two keys share the same text.

**What about dynamic texts (`Hello {name}`)?** ICU placeholders act as wildcards when matching, so `Hello Ada` finds `greeting`.

**My site is behind a login.** Reviewers log in as usual; the token survives navigation within the tab.

**Can a reviewer break a translation?** Only `edit` links and members can apply a text, and every write goes through the validator. Each change is versioned and can be reverted from the key's history.

**Can I turn review off?** Revoke the links, or remove the origin in **Review → Settings**: the widget then refuses to start.

**How many annotations can I create?** Free: 200 per month, Pro: 10,000, Team: unlimited. Self-hosted: unlimited.
