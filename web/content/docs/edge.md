# glot Edge (SEO localization)

glot Edge serves each language of your site as **translated HTML on its own URL**. The first HTML byte is already translated, so search engines index it without running JavaScript. You don't need i18n keys in your code: glot translates the rendered page segment by segment, and the keys from your glot catalogs still take precedence wherever they exist.

> **Note:** glot Edge is part of the Growth plan. Pro includes a limited trial (one domain, a capped number of edge pages per month). A self-hosted server has no limit.

## Two modes: dynamic or SEO

glot offers two ways to put a language in front of visitors. They use the same catalogs, glossary and validation.

| | Dynamic localization | SEO localization |
|---|---|---|
| How | CDN catalogs + runtime (`/cdn/{slug}/{locale}.json`, `<glot-locale-switcher>`) | glot Edge (proxy, worker or middleware) or pre-rendering from your CI |
| Deploy needed for a new translation | No | No |
| URL per language | No (same URL) | Yes (`es.example.com` or `example.com/es/`) |
| Indexed by search engines | Mostly the source language | Every language: translated HTML, hreflang, canonical |
| Works without JavaScript | No | Yes |
| Keys in the code | Yes | No (keys are used when present) |

Choose **Dynamic localization** for an app behind a login, where SEO doesn't matter. Choose **SEO localization** for marketing pages, docs, pricing, blogs: anything you want people to find in their own language.

## How it works

1. A request reaches glot Edge for `es.example.com/pricing`.
2. glot fetches the page from your origin. The origin is fixed per domain and never taken from the request.
3. The HTML is cut into segments: the text of each block element, with inline tags (`<a>`, `<strong>`…) kept as numbered placeholders. Translatable attributes (`title`, `alt`, `aria-label`, `placeholder`, meta description, Open Graph) are segments too.
4. Each segment is looked up in the project's translation memory, by text **and context**: the element carrying it (`button`, `a`, `h1`, `title`…) and a page template derived from the URL. "Save" on a button and "Save 20 %" in a price are different segments.
5. The page is served translated, with `<html lang dir>`, `Content-Language`, hreflang and a canonical URL for that language.

A segment glot has never seen is served **in the source language**: a page never has a hole. glot reports it (text, context and path only), translates it with your model, glossary and validators, and serves the translation from the next request on. Coverage grows with real traffic.

## Set up a domain

Open your project, then **glot Edge → Setup**, and pick a serving mode.

### Subdomain (hosted by glot)

`es.example.com` points to glot with a CNAME. glot issues the HTTPS certificate on demand, only for domains verified in a project.

```txt
es.example.com.        CNAME  edge.glotlang.com.
_glot.es.example.com.  TXT    "<token shown in the dashboard>"
```

Click **Check DNS**. As long as the domain is not verified, glot serves nothing for it.

### Subdirectory

`example.com/es/` goes through a Cloudflare Worker (`@glot-i18n/edge`) or a proxy rule on your host. Only the TXT record is needed to verify the domain. The dashboard shows the configuration for your project.

### Middleware

Everything stays on your hosting: a Next.js or Express middleware from `@glot-i18n/edge` translates responses before they leave your server.

### Pre-rendering from your CI (no proxy)

If you'd rather not put anything in front of your site, glot can trigger your build when a language changes and is ready (GitHub `repository_dispatch` through the GitHub App, Vercel or Netlify hooks, or a signed generic webhook). Your CI then produces the translated HTML with hreflang from its own pages.

## Check it as Google does

**Setup → View as Google** fetches one translated URL once, without JavaScript: HTTP status, `lang` and `dir`, `Content-Language`, title, canonical, hreflang (and whether each one links back), `Server-Timing`, segments still untranslated, and the start of the raw HTML.

The **SEO report** tab does the same for every language and page: raw HTML without JavaScript, canonical, reciprocal hreflang, metadata, untranslated segments, HTTP errors, and the difference between the raw HTML and the page rendered in a browser.

The report separates two things:

- **Technically indexable**: what glot measures (status 200, no `noindex`, self canonical, text translated in the raw HTML).
- **Actually indexed**: only Google knows. It is shown once Search Console is connected, and stays **unknown** otherwise. glot never guesses it.

### Sitemaps and IndexNow

glot publishes one sitemap per language plus a sitemap index, and announces them in `robots.txt`. Google discovers the localized URLs from the sitemaps and the hreflang links. IndexNow pings are sent to Bing and Yandex only: Google does not use IndexNow. A language is listed in hreflang links and sitemaps only once it is ready (95 % coverage and every critical page above its threshold). Until then its pages are served translated, without hreflang or canonical, so search engines don't index a half-translated version.

## Coverage by critical page

"95 % of keys" is not "95 % of the experience". glot Edge measures coverage on the pages actually served: translated segments / rendered segments, per URL.

Some pages are **critical**: home, sign-up, pricing, checkout by default (from your sitemap and URL patterns, editable in **Settings**, or with the star in **Coverage**). A language is offered in the switcher and marked `ready` in `/cdn/{slug}/locales.json` only once every critical page reaches the threshold (98 % by default).

In **Coverage**, each page lists its missing segments; click one to open it in **Keys → Segments**.

## Review and editing

**Keys → Segments** shows the segments next to your keys: source text, context, where it was seen, how often, and the translation for each language. Edits are validated like keys (syntax, glossary, length); an invalid translation is refused with the reason.

New segments first seen on **sensitive pages** (payment, account, settings by default) are not published automatically: they wait in **glot Edge → Review queue** with the status *to review*, and the source text is served until someone approves them.

## Discovery zones

**glot Edge → Settings** limits where glot may learn new text:

- **Allowed and excluded paths**: path patterns (`/blog/*`). Empty allowed list = every public page.
- **Translated areas and excluded selectors**: CSS selectors. Excluded areas are never translated nor sent to glot (user content, names, comments, account widgets).
- **New segments per day**: beyond the cap, new text is served in the source language until the next day.
- **Automatic translation** on or off, and a monthly budget.
- **Rendering**: translate the origin HTML, a pre-rendered snapshot, or automatically use a snapshot when the origin returns an empty shell (SPA).

## Security and privacy

- **Never an open proxy**: glot only serves verified domains of a project, each with a fixed origin. Private IP ranges are refused; responses are limited in size and time.
- **No visitor data**: glot stores segment texts (page copy), their context and paths without query strings. No IP, no cookie, no user identifier.
- **Never behind a login**: when a request carries a session cookie, nothing is discovered.
- **Protected content** is never translated or reported: code, `pre`, `kbd`, `[translate=no]`, `.notranslate`, `[data-glot-skip]`, `[data-glot-user]`, form values (`input`, `textarea`, `select`, `contenteditable`), and do-not-translate glossary terms.
- **Personal data detection**: a segment that looks like an email, phone number, IBAN or card number, address, long number, UUID or token is never sent to glot.
- **Your HTML stays yours**: only segment text changes. Security headers from your origin are kept.

## Availability and fallback

glot is never a reason for a broken page.

- **Fail-open**: if glot doesn't answer within a short timeout, the visitor gets your original page, untranslated. glot never returns a 5xx of its own.
- **Stale content**: responses are cached per URL, language and catalog version, with `stale-while-revalidate` and `stale-if-error`. The worker and proxy keep the last known translation memory in memory and on disk.
- **One version everywhere**: each language has one monotonic version shared by the CDN catalogs, the edge translation memory, pre-rendered snapshots and the browser patcher. A page never mixes two versions, and the title Google sees and the one a visitor sees come from the same version. Changing a translation in glot bumps the version and purges the cache: no redeploy.
- **glot itself down (subdomain mode)**: fallback to your origin when glot doesn't answer. Caddy, in front of glot, keeps a map of your verified subdomains and their origins (rewritten by glot on every domain change); if the glot server doesn't answer, Caddy serves your origin directly, untranslated, with `x-glot-edge: fail-open; glot=down`.
- **Origins that send `Cache-Control: no-cache`**: glot still asks your origin on every request, but when the HTML is byte-for-byte the same as last time, it reuses the translation instead of translating again (`x-glot-edge: translated; cache=hit-body`).
- **Kill switch**: the switch at the top of **glot Edge** turns it off for the project (each domain also has its own). Within a minute, every domain serves your origin unchanged.
- **In the browser**: the catalog loader and the language switcher keep the last version in local storage and use it when the CDN doesn't respond.
- **Zero dependency at serving time**: `glot pull` writes your catalogs into your repository, and the edge translation memory of a language is a plain JSON file (`GET /edge/v1/{project}/{locale}/tm`) you can save next to them and serve yourself.

Performance is visible in **glot Edge → Performance**: pages served, cache hit ratio, fail-open count, and glot's own cost on top of your origin (`Server-Timing`).

## Single-page apps and hydration

If your origin returns an empty or skeleton HTML (client-side rendering), glot translates a snapshot rendered by a headless browser instead. Snapshots are cached and refreshed on deploy, from the sitemap or on demand; a page is never rendered per request.

Frameworks that render on the server then **hydrate** in the browser compare the HTML with the data they serialized. glot handles this in two ways:

- **Serialized payloads are translated too** when their strings exactly match segments (`__NEXT_DATA__`, `__NUXT__`, React Server Components flight data), so the browser hydrates on the same text.
- **A small patcher** (`/__glot/patcher.js`, served from your own domain, 6.4 kB gzip, loaded `async`) re-applies the translation memory after hydration and on every re-render, in the same task as the change (no painted flash). It walks the live DOM with the server's segmentation rules (same segment text, same hash), rewrites text nodes in place (framework-owned nodes and listeners survive) and ignores a translation memory older than the page.
- **Text that only exists in the browser** (a label your JavaScript copies from a `data-*` attribute into a button, a widget rendered after load) is reported by the patcher to `/__glot/missing` on your domain: batched, once per page view, never when the visitor has a session cookie (your middleware re-checks HttpOnly cookies), never personal data or protected areas. glot translates it like any new segment, and the patcher picks up the new version on its next poll (30 s, 2 min, 10 min) and applies it in place. `data-*` attributes themselves are never translated; only `title`, `alt`, `aria-label`, `aria-description`, `placeholder`, button `value` and SEO `meta` contents are.

### Limits by framework

What we tested end to end (real builds, headless Chromium: hydration errors, painted frames, final text):

| Framework | Result |
|---|---|
| Static HTML, server templates (Rails, Django, Laravel, PHP…) | Translated HTML served directly, no hydration involved. |
| Next.js 16, App Router and Pages Router | **Pass** with payload translation (`__NEXT_DATA__`, RSC flight data) + patcher: no hydration error, no flash. |
| Next.js, strings hard-coded in client components | No visible flash (the patcher re-applies the translation before paint), but React may still log one recoverable hydration error (#418) for that subtree. Move the string to a server component or a glot key to remove it. |
| Leptos CSR (ClipOnIt) | **Pass** with the patcher: the app re-renders in the source language and the patcher translates it before paint. |
| Nuxt 3/4 | `__NUXT_DATA__` translation is unit-tested only; no end-to-end run yet. |
| SvelteKit | Its serialized data is JavaScript, not JSON: not translated. The patcher still re-applies the translation memory after hydration. |
| React 18/19 SSR outside Next.js | Not tested. |
| Client-side SPA (React, Vue…) with an empty HTML shell | Pre-rendered snapshot + patcher. |

- **Subdomain mode** needs glot in the request path. If the glot server doesn't answer, Caddy falls back to your origin (untranslated, `x-glot-edge: fail-open; glot=down`) for verified subdomains with an `https` origin; a plain-`http` origin has no fallback.
- **hreflang lists ready languages only** (those `/cdn/{project}/locales.json` reports as ready): a language still being translated is served under its URL but not advertised to search engines until it is ready.
- Text built in JavaScript from several pieces ("You have " + n + " files") can't match a segment; use a glot key for it.

## Compared with other approaches

**Key-based i18n** (next-intl, i18next, glot catalogs): precise, reviewable, works offline, and remains the best choice for app UI. It needs keys in the code and a build or CDN per language; on a site that was never internationalized, that's a rewrite. glot Edge covers the text without keys today, and uses your keys where you add them.

**Translation proxies** (the "translate my site by DNS" products): no code changes, like glot Edge. They usually translate without your glossary or validation, give little control over what gets sent, often break single-page apps at hydration, and treat SEO as a checkbox. glot Edge validates every translation with your project's rules, never sends protected areas or personal data, publishes nothing new on sensitive pages without a review, measures coverage on the pages that matter, and reports what is indexable instead of claiming it.

What glot Edge does not do: translate images or text inside canvas or video, or know whether Google actually indexed a page without Search Console.
