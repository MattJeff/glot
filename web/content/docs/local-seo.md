# Local SEO

A translated keyword is rarely what people type. A French page titled "Recadrez vos vidéos" becomes "Recorta tus vídeos" in Spanish, but people in Madrid search "recortar vídeo online" and people in Mexico City "cortar video en línea". Local SEO adapts the **search-facing** content of each page to each market, and leaves the body a faithful translation:

- `<title>` and meta description (with Open Graph and Twitter copies)
- the main heading (H1) and the H2s
- a URL slug **suggestion**
- FAQ structured data (`FAQPage` JSON-LD)

It requires the [self-hosted server](self-hosting.md) or glot cloud. It works best with [glot Edge](edge.md), which serves the result; without Edge, the [website tag](website.md) can read the same data.

## A market is a locale, not a language

`es-ES` and `es-MX` are two markets: separate search data, separate proposals, separate approvals. A project that only targets `es` gets one proposal for Spanish; if you later add `es-MX`, it uses the `es` content until its own is approved. A region never borrows from a sibling region (`es-MX` never shows what was approved for `es-ES`).

Each market follows the length budget of its main search engine: Google (title 60, description 160), Baidu for `zh-CN`, Yandex for Russian, Naver for Korean. Chinese, Japanese and Korean characters count double.

## Where the search data comes from

glot **never invents a keyword or a volume**. A market without data says "No search data yet" and proposes nothing.

| Source | Cost | What it gives |
|---|---|---|
| Your own keyword list | free | Paste the queries you already know per market, with their volume if you have it. |
| Google Search Console | free | The queries your pages already appear for, per country, with impressions, clicks and position. Read-only access that you grant with your own Google account. |

Search Console only knows queries your site already shows up for. In a market you have never ranked in, it returns nothing: start with your own list, and Search Console takes over as impressions arrive.

Not used, on purpose: Google Autocomplete (an unpublished endpoint; automated queries are against Google's terms) and Google Trends (its official API is an invitation-only alpha). Paid keyword databases (DataForSEO, Semrush, Ahrefs…) can be added as adapters with your own key; none is called today.

### Connect Search Console

In **Local SEO**, click **Connect Search Console** and accept the read-only request with a Google account that owns the property. glot picks the property that covers your site (the domain property first).

Self-hosting: create an OAuth client (type "Web application") in Google Cloud, enable the *Google Search Console API*, add the redirect URI `https://YOUR_GLOT_DASHBOARD_HOST/api/local-seo/gsc/callback` (the dashboard origin, `GLOT_WEB_ORIGIN`: the callback needs your session), then set:

```sh
GOOGLE_OAUTH_CLIENT_ID=...
GOOGLE_OAUTH_CLIENT_SECRET=...
```

Without these variables the dashboard says Search Console is not available on this server, and your own keyword list still works. The refresh token is stored encrypted and never returned by the API.

## Scan

**Scan pages** reads your public pages (raw HTML, anonymously: never a logged-in page), then for each page × market:

1. **Audits what the market shows today**: too long for the search engine, same title as another page, brand name lost, title still in the source language. This needs no search data.
2. Collects the market's search terms from your sources.
3. When terms exist, asks your project's LLM for a title, description, H1/H2, slug suggestion and up to three FAQ entries built around them.

What is sent to the LLM: the page's title, description, headings and a 2 000-character text excerpt, the search terms, and your glossary. Nothing about visitors.

## Checks every proposal must pass

The same checks run on the model's proposal and on your own edits. Errors block approval.

| Check | Rule |
|---|---|
| Length | Title and description fit the market's search engine. |
| No keyword stuffing | A search term appears once in the title; no keyword lists. |
| No claim the page does not make | "free", "best", "official", "guaranteed", discounts, and every figure must be stated on the page. People may search "gratis"; if your page is not free, the title cannot say it. |
| Brand terms | Glossary terms marked *do not translate* keep their exact spelling. |
| Uniqueness | No two pages of a market share a title, description or slug. |
| Regions | A warning when `es-MX` gets the same title as `es-ES` although people search different words. |

## Approve, edit, or automate

By default **nothing goes live without your approval**. Each row shows what the market displays today, the proposal, the search terms behind it and their source. You can approve, edit (your version replaces the proposal and is checked again), or reject. Rejecting a live override takes it down.

**Auto mode** is an explicit switch for admins: proposals that pass every check with no warning go live on their own; anything with an error or a warning still waits for you.

If the source page's title, description or H1 changes, or the page stops supporting a claim, the override is taken down ("Page changed") until someone approves a new one.

## Measure

With Search Console connected, **Update search data** stores daily impressions, clicks and average position for each page × market that has an override, and the dashboard compares the 28 days before with the days since it went live. Without stored data the dashboard shows no number.

## For developers

Approved overrides of a market, for the website tag or your own code:

```
GET /cdn/{project}/local-seo/{locale}.json
→ { "locale": "es-MX", "pages": { "/pricing": { "title": "…", "description": "…", "h1": "…", "h2": [], "faq": [] } } }
```

API (session cookie or `Authorization: Bearer glot_…`, admin role to change anything):

| Call | Purpose |
|---|---|
| `GET /api/projects/{slug}/local-seo` | Everything the dashboard shows |
| `POST …/local-seo/scan` `{paths?, pages?, locales?, regenerate?}` | Scan (at most 10 pages per call). `pages` gives content directly when the raw HTML is empty before JavaScript. |
| `PUT …/local-seo/demand` `{locale, path?, terms: [{term, impressions?}]}` | Replace your keyword list for a market (or one page) |
| `POST …/local-seo/overrides/{id}/approve` · `/reject` | Decide |
| `PATCH …/local-seo/overrides/{id}` `{content, approve?}` | Edit |
| `PUT …/local-seo/settings` `{mode: "review" \| "auto", site_url?}` | Consent mode, site URL |
| `POST …/local-seo/metrics/refresh` | Pull Search Console numbers |

## Limits today

- The slug is a suggestion: changing URLs stays a decision you take in glot Edge's translated slugs.
- FAQ is added as structured data only, not as visible content.
- Image alt texts are not adapted yet.
- Bing Webmaster Tools is not connected yet.
