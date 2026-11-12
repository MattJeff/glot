# Sentences everywhere (text graph)

> **Experimental.** This is a Labs feature of the self-hosted server and the cloud dashboard. The API may change.

Translation tools manage files of keys, one project at a time. Your company owns sentences, and the same sentence lives in several places: a key in your code, a page of your website, a product title in your CMS, an email. The text graph gives each sentence one identity and records every place it appears, so that a translation fixed once applies everywhere, and a sentence translated two ways is found.

Open **Sentences** in a project. There is nothing to configure: glot reads the stores it already has.

| Surface | What glot reads | A place is |
| --- | --- | --- |
| Code | The catalog keys you push (`glot push`, the GitHub app). Mobile OTA apps use the same keys. | a key |
| CMS | The fields mirrored by a [connector](integrations.md) (Shopify, WordPress, Contentful…). | a field of an entry |
| Website | The segments [glot Edge](edge.md) and the [website tag](website.md) saw on your pages. | a text on a page template |
| Anything else | What you declare with the [importer contract](#importer-contract): emails, design files, PDFs. | whatever address you give |

## What "the same sentence" means

A sentence is identified by its text and its context, never by a key name. Renaming a key, moving it to another file or deleting and recreating it changes nothing: the sentence keeps its identity and gets its translations back.

- **Whitespace, quote style and non-breaking spaces are ignored.** `Don’t wait…` and `Don't wait...` are one sentence.
- **Placeholder syntax is ignored.** `Hello {name}`, `Hello {{name}}`, `Hello %s`, `Hello %1$s` and `Hello %{name}` are one sentence. When a translation is copied from one place to another, its placeholders are rewritten for the destination: `Bonjour {name}` becomes `Bonjour %s`. If that cannot be done exactly (different number of placeholders, a placeholder the source does not have), nothing is copied and the reason is shown.
- **Wording is not ignored.** `Save changes` and `Save changes.` are two sentences. They are shown to each other as *related* (same words), and so are sentences that are nearly the same. Related sentences are never merged or translated from one another.
- **Context keeps short texts apart.** A text of three words or fewer on a button or a link is a different sentence from the same text used elsewhere: "Save" the action is not "Save" the discount. Each one lists the other as *same text, other context*. Longer sentences mean the same thing wherever they are.

ICU plural and select messages match only messages with the same syntax.

## Translate once, apply everywhere

**Fill N places** copies, for every sentence that appears in several places, the translation nobody disputes to the places that lack it. The rules are fixed:

1. If exactly one human-reviewed translation exists, it wins. It replaces machine translations of the same sentence and fills the empty places, and it stays *reviewed* where it lands.
2. If no translation is reviewed and every place agrees, that machine translation fills the empty places as a machine translation.
3. A machine translation never replaces a reviewed one.
4. Two machine translations that disagree are an **inconsistency**. Two reviewed translations that disagree are a **conflict**. Neither is settled automatically: both are listed under *Translated two ways* until a person chooses.
5. Every copy passes the validation of the place it lands in (placeholders, message syntax, glossary). A copy that fails is skipped and the reason is shown.
6. A page glot Edge already serves from the catalog key of the same text is left alone: it follows the key.

**Use everywhere** on a translation is the one action that settles a sentence. A preview says how many places will change before anything is written. Places holding a different human-reviewed translation are kept unless you tick *Replace human-reviewed translations too*.

Translations are written where they already live: catalog translations (with "text graph" as author in the key's history), glot Edge translations, and through the catalog to your CMS on the connector's next push.

## When a source changes

If a sentence is reworded in one place (a key's source changed) while other places still show the old wording, the page lists it under *Reworded in one place only*, with the places that were left behind.

## API

All routes are under `/api/projects/{slug}/text-graph` and accept a session cookie or an API token (`Authorization: Bearer glot_…`).

| Route | Does |
| --- | --- |
| `GET /` | Counts, sentences translated two ways, what *Fill* would write. |
| `GET /search?q=&limit=` | Sentences whose text, key or page path contains `q`. |
| `GET /anchors/{id}` | One sentence: every place, translations per language, related sentences. |
| `POST /anchors/{id}/unify` | `{locale, value?, force?, dry_run?}`: one translation in every place. |
| `POST /propagate` | `{dry_run?}`: apply the rules above to the whole project. |
| `POST /sync` | Re-read the catalog, CMS and website places now (reads do it when older than a minute). |
| `POST /import`, `GET /export` | The importer contract below. |

A sentence `id` is a hash of the project's source language, the normalized text and the context. It is the same on every server and after any rebuild.

## Importer contract

Any system that shows text can join the graph. It declares its texts and where they are; glot answers with the sentence each one is. Admin role.

```http
POST /api/projects/my-app/text-graph/import
Authorization: Bearer glot_…
Content-Type: application/json

{
  "surface": "email",
  "replace": true,
  "items": [
    {
      "locator": "welcome/subject",
      "text": "Hello %s, welcome back",
      "role": "heading",
      "context": null,
      "url": "https://mail.example.com/templates/welcome",
      "meta": { "template": "welcome" },
      "translations": { "fr": { "value": "Bonjour %s, bon retour", "reviewed": true } }
    }
  ]
}
```

| Field | Required | Meaning |
| --- | --- | --- |
| `surface` | yes | Your name for the system: 2 to 32 characters of `a-z`, `0-9`, `-`, `_`. `catalog`, `connector` and `edge` are reserved. |
| `replace` | no | `true`: this payload is the whole surface, places it does not list are removed. Default `false` (add or update). |
| `items[].locator` | yes | The stable address of the text in your system (a template path, a Figma node id). Unique in the payload, at most 500 characters. |
| `items[].text` | yes | The text in the project's source language. |
| `items[].role` | no | What carries the text: `button`, `link`, `heading`, `label`… Only used to keep short action texts apart. |
| `items[].context` | no | An explicit meaning, like gettext's `msgctxt`. Texts with different contexts are never the same sentence. |
| `items[].url` | no | Where a person can see this text. Shown as a link in the dashboard. |
| `items[].meta` | no | Any JSON object you want back. |
| `items[].translations` | no | Per target language: a string, or `{ "value", "reviewed" }`. A reworded text drops the translations stored for its old wording. |

At most 5000 items per request. The response gives, per item, the `anchor_id` of its sentence and `elsewhere`: how many places outside your surface already show it.

To read what your surface should show, without writing anything:

```http
GET /api/projects/my-app/text-graph/export?surface=email&locale=fr
```

```json
{
  "surface": "email",
  "locale": "fr",
  "strings": {
    "welcome/subject": { "value": "Bonjour %s, bon retour", "status": "reviewed", "from": "own", "anchor_id": "5f0c…" },
    "welcome/cta": { "value": "Commencez votre essai gratuit", "status": "reviewed", "from": "graph", "anchor_id": "91ab…" }
  },
  "missing": ["welcome/footer"]
}
```

`from: "graph"` means the translation comes from another place of the same sentence, rewritten for your placeholders. `missing` lists the texts no place has translated yet, or that are translated two ways.

## Privacy

The graph holds text your project already stores: key sources, CMS fields and page segments. Building it sends nothing to an LLM or to any third party, and uses no visitor data. Website places carry a page path without its query string and a view counter, both already recorded by glot Edge.

## Limits

- Matching is lexical. Two sentences that mean the same with different words are not linked; paraphrase detection with embeddings is designed but not built.
- A sentence is searched by its source text, a key or a path, not by its translations.
- Laravel `:name` placeholders are not recognized as placeholders.
- The graph is per project. Sentences are not shared between projects.
