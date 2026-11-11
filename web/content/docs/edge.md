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

For an app that ships an empty HTML shell, your CI can also render its pages itself and send them to glot Edge (`glot prerender --upload`): see [100 % JavaScript apps](#100-javascript-apps).

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

## Application pages (sign-in, OAuth, API)

Your app's own pages must stay on your origin domain: the session cookie is set for that host only, and OAuth providers (Google, TikTok, Stripe…) only accept the redirect URIs registered there. List them in **glot Edge → Settings → Application paths**, one rule per line. glot Edge never translates, caches or discovers them.

```
/login
/dashboard
/live/
redirect /settings
proxy /api/
proxy /assets/
```

- **Matching by segment**: `/explainer` matches `/explainer` and `/explainer/edit`, not `/explainer-video`. A trailing slash means sub-pages only: `/live/` matches `/live/abc`, not `/live`. `*` matches anything: `/x*` is a raw prefix. Matching ignores case and runs on the normalized path (`//`, `.`, `..` and encoded dots resolved).
- **`redirect`** (default): `302` to the same page on your origin domain, with the original query string (`307` for forms and other non-GET requests, so the method and body are kept). The language is added the way your site expects it, from the URL pattern of the project's preview settings: `{route}?lang={locale}` → `?lang=es` added (or replaced), `/{locale}{route}` → `/es/…`, otherwise nothing. `es.cliponit.app/login?next=/x` → `cliponit.app/login?next=/x&lang=es`.
- **`proxy`**: the origin's response passed through untouched (cookies included), untranslated, never cached, never discovered. Use it for the API, assets and technical files, so a translated page loads its resources without a detour.
- **Default action**: the action of lines that don't start with `redirect` or `proxy`.
- New projects start with a suggested list (`/login`, `/register`, `/dashboard`, `/auth/`, `/oauth/`, `/settings`, `/account`, `/checkout`, `/billing`, `proxy /api/`) that you can edit.
- **Always glot's**: `/sitemap.xml`, `/robots.txt` and the IndexNow key file on the edge host are generated by glot for that language and are never redirected nor passed through, whatever the rules.
- **Safety**: redirects only go to the domain's own verified origin, never to a host taken from the request; `//evil.com`-style paths stay paths on your domain. If your origin sends the visitor back to the edge host for the same page, glot passes the page through instead of redirecting again (no loop).
- Responses that aren't HTML are never translated. Requests with a session cookie are never cached or discovered; whether they are translated is your choice (see [Pages behind sign-in](#pages-behind-sign-in)).
- The Worker, Next.js proxy and Express middleware read the same rules from glot (`/edge/v1/{project}/status`) and apply them within their refresh interval (20 s).

## Pages behind sign-in

Dashboards, account pages and invoices can be translated too, without glot ever seeing your users' data and without touching their session. Choose how in **glot Edge → Settings → Pages behind sign-in**. A request "has a session" when it carries an `Authorization` header or a cookie whose name contains `sess`, `auth`, `token`, `sid`, `jwt`, `login`, `logged`, `user` or `account`.

| Setting | What signed-in visitors get | What glot receives |
|---|---|---|
| **By Edge, private response** (default) | The page translated by the Worker or Next.js proxy on your domain, from the translation memory | Nothing about the page or the visitor |
| **In the browser (script tag)** | Your page untouched from the server; the `data-glot-app` tag translates it in the browser | The translation memory request only (no cookie). With discovery on: interface text, see below |
| **Don't translate** | Your page, in the source language | Nothing |

### By Edge, in a subdirectory of your own domain

With the Cloudflare Worker or the Next.js proxy, `/es/dashboard` is served by the same host as `/dashboard` (`cliponit.app/es/…`), so the browser sends your session cookie as usual:

- The request reaches your app as is, cookies included, and your `Set-Cookie` headers come back as is. Your app's own `fetch` calls (`/api/me`) keep working: they are on the same domain.
- The page is translated for that response only: `Cache-Control: private, no-cache` and `Vary: Cookie`, whatever your app sent. It never enters the Worker's page cache or any shared cache.
- Nothing is discovered or reported. Translating happens inside the Worker or proxy from a translation memory it already holds; no part of the page leaves your infrastructure. Text that has no translation stays in the source language.
- Application paths (see above) take precedence: a path listed there, like the suggested `/dashboard`, is redirected to your origin domain, never translated by Edge. Remove it from the list to have Edge translate it, or use the browser mode there.
- Subdomains (`es.example.com`) are a different host: your session cookie only reaches them if you set it for the parent domain. Prefer a subdirectory for signed-in pages.

### In the browser, with one script tag

Add this to the pages of your app (or its layout), and pick **In the browser** in the settings:

```html
<script async src="https://glotlang.com/widget/glot-app.global.js" data-glot-app
  data-project="your-project" data-server="https://api.glotlang.com"
  data-source="en" data-locales="es,de" data-key="pk_…"></script>
```

- It runs on your own domain and proxies nothing, so the session is never touched. It never reads, sets or sends your cookies; requests to glot are cross-origin and without credentials. It only writes `glot_locale`, the visitor's language choice.
- It applies the project's translation memory to text as your app renders it, in place (framework-owned nodes and listeners survive), like the Edge patcher.
- **Language**: `?lang=es` (what application-path redirects add), then the visitor's saved choice, then the browser's language. A language menu appears in a corner of the page, or inside an element with `data-glot-switcher`. With `data-switcher="none"`, build your own and call `glotApp.setLocale("es")`.
- It only works when the project's CDN is public (the translation memory is fetched from glot by the browser) and does nothing unless the setting is on, so you can switch it off from glot without a deploy.
- There is no server pass: on the first load, text shows in the source language until the translation memory arrives (cached by the browser afterwards).

### Discovery in the browser mode (off by default)

**Discover interface text** lets the script report text that has no translation yet. It is limited on purpose:

- Only interface elements are candidates: `[data-glot-ui]`, `nav`, `button`, `label`, headings, `th`, `legend`, `summary`, `title` and the `button`, `tab` and `menuitem` roles.
- Only text inside `[data-glot-ui]` is sent as text. For any other candidate, the script sends its fingerprint alone, and only when the translation memory already contains that text (a text glot learnt from your public pages, now seen in a new context). A user's name, never seen on a public page, is never sent, not even as a fingerprint.
- Form values, `[data-glot-user]`, your excluded selectors and anything that looks like personal data (emails, phone numbers, amounts and other numbers, IBAN, tokens) are never sent. glot checks again on its side and refuses text longer than an interface label.
- The URL is stored as a page template (`/invoices/:id`), never with ids or query strings.
- Every new text waits in **Segments → pending** until someone reviews it; it is never served before.

Mark user content explicitly with `data-glot-user` (names, messages, file names) so it is never translated or reported, whatever the mode.

## Security and privacy

- **Never an open proxy**: glot only serves verified domains of a project, each with a fixed origin. Private IP ranges are refused; responses are limited in size and time.
- **No visitor data**: glot stores segment texts (page copy), their context and paths without query strings. No IP, no cookie, no user identifier.
- **Never behind a login**: when a request carries a session cookie, nothing is discovered (the opt-in browser mode below only reports interface text, and every new text waits for a review).
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

- **Serialized payloads are translated too** when their strings exactly match segments, so the browser hydrates on the same text: Next.js `__NEXT_DATA__` and React Server Components flight data, Nuxt `__NUXT_DATA__`, SvelteKit's `kit.start(…)` data, SolidStart's serialized resources (`self.$R`), React Router 7 / Remix loader data (`__reactRouterContext` stream) and Astro island props (`<astro-island props>`). Strings are edited in place (key order, numbers and the rest of the script stay byte for byte); object keys, `x["…"]` lookups and values of identifier-like keys (`id`, `type`, `variant`, `plan`…) are never touched, and a script glot can't read with certainty (template strings, regexes, comments) is left as is. On Vue pages (Nuxt, Astro Vue islands) glot also sets `<html data-allow-mismatch="text">`: Vue then replaces a hard-coded text it finds translated without logging a hydration error, and the patcher re-translates it in the same task.
- **JavaScript chunks are translated too** (bundle localization): text hard-coded in client code (`<p>Cancel anytime</p>` in a client component compiles to the string `"Cancel anytime"` in a chunk) is rendered by the browser from the chunk, not from the HTML. glot parses each chunk the page loads (a real JavaScript parser, so comments, regexes and templates are never confused with text) and replaces string literals that are exactly a segment already seen in your rendered pages and translated the same way everywhere. Never touched: object keys, `import`/`require` paths, `"use client"`-style directives, values compared with `===` or used in `case`, `x["…"]` lookups, function arguments (`track("Add to cart")`), template strings with `${…}`, and values of identifier-like keys (`id`, `type`, `variant`…). A chunk keeps its URL; since its content now depends on the language and the translation version, glot serves it with `Cache-Control: public, no-cache` and a strong `ETag` instead of `immutable`: the browser keeps it and revalidates with a `304`, and a translation edited in glot reaches the chunk and the HTML together. Subdomain mode does this in the proxy; the Cloudflare Worker (one URL for every language) picks the page's language from the `Referer` and gets the rewritten chunk from glot (`POST /edge/v1/{project}/{locale}/bundle` with your public key), cached per URL, language and version.
- **A small patcher** (`/__glot/patcher.js`, served from your own domain, 6.4 kB gzip, loaded `async`) re-applies the translation memory after hydration and on every re-render, in the same task as the change (no painted flash). It walks the live DOM with the server's segmentation rules (same segment text, same hash), rewrites text nodes in place (framework-owned nodes and listeners survive) and ignores a translation memory older than the page.
- **Text that only exists in the browser** (a label your JavaScript copies from a `data-*` attribute into a button, a widget rendered after load) is reported by the patcher to `/__glot/missing` on your domain: batched, once per page view, never when the visitor has a session cookie (your middleware re-checks HttpOnly cookies), never personal data or protected areas. glot translates it like any new segment, and the patcher picks up the new version on its next poll (30 s, 2 min, 10 min) and applies it in place. `data-*` attributes themselves are never translated; only `title`, `alt`, `aria-label`, `aria-description`, `placeholder`, button `value` and SEO `meta` contents are.

### Limits by framework

What we tested end to end (real builds, headless Chromium: hydration errors, painted frames, final text):

| Framework | Result |
|---|---|
| Static HTML, server templates (Rails, Django, Laravel, PHP…) | Translated HTML served directly, no hydration involved. |
| Next.js 16, App Router and Pages Router | **Pass** with payload translation (`__NEXT_DATA__`, RSC flight data) + patcher: no hydration error, no flash. |
| Next.js 16, strings hard-coded in client components or Pages Router JSX (webpack and Turbopack builds) | **Pass** with bundle localization: no hydration error (#418), no flash, page translated. Without it (JavaScript served from your origin): one recoverable #418 per page, and the patcher has to win a race against React's client re-render (we saw one painted flash in 6 runs). |
| Vite + React 19 SSR (`hydrateRoot`), every string hard-coded | **Pass** with bundle localization: no hydration error, no flash. |
| Leptos CSR (ClipOnIt) | **Pass** with the patcher: the app re-renders in the source language and the patcher translates it before paint. |
| SvelteKit 2 (Svelte 5) | **Pass**: `load()` data translated in `kit.start(…)`; text hard-coded in components and text rendered after mount: no error, no flash. |
| Nuxt 4 (Vue 3.5) | **Pass**: `__NUXT_DATA__` translated; hard-coded component text replaced silently by Vue (`data-allow-mismatch`) and re-translated by the patcher before paint. Without glot's handling (a plain translating proxy): "Hydration completed but contains mismatches". |
| SolidStart 2 | **Pass**: serialized resources translated; hard-coded and after-mount text: no error, no flash. |
| React Router 7 (Remix) | **Pass** when visible text comes from loaders (stream payload translated; a plain proxy gets #418). Text hard-coded in components: one recoverable #418, no flash, unless bundle localization serves the chunks. |
| Astro 7 islands (React 19, Vue 3.5) | **Pass** for island props and Vue islands (hard-coded text included). A React island with hard-coded text: one recoverable #418 for that island, no flash, unless bundle localization serves the chunks. |
| React 18/19 SSR outside a framework | See Vite + React above. |
| Client-side SPA (React, Vue…) with an empty HTML shell | Pre-rendered snapshot + patcher. |

- **Subdomain mode** needs glot in the request path. If the glot server doesn't answer, Caddy falls back to your origin (untranslated, `x-glot-edge: fail-open; glot=down`) for verified subdomains with an `https` origin; a plain-`http` origin has no fallback.
- **hreflang lists ready languages only** (those `/cdn/{project}/locales.json` reports as ready): a language still being translated is served under its URL but not advertised to search engines until it is ready.
- **Text rendered after load** (after mount, client-only): a translation memory up to 32 kB rides in the page (`<script type="application/json" id="glot-tm">`), so the patcher translates it before it is painted, even on a first visit. A larger memory is fetched by the patcher: on a fast connection the app can paint such a text in English for a frame before it arrives (we saw it in 2 of 5 cold runs on SvelteKit without the inline memory).
- **Data loaded on client-side navigation** (SvelteKit `__data.json`, Nuxt `_payload.json`, React Router `.data`, server functions) and **streamed promises** (React Router deferred data, SvelteKit streamed `load` promises) are not rewritten: the patcher translates what they render, and a page you open directly is fully translated in its HTML.
- Text built in JavaScript from several pieces ("You have " + n + " files") can't match a segment; use a glot key for it.
- **Bundle localization** only rewrites a string that glot has already seen rendered in one of your pages and translated the same way in every context; a text with two meanings ("Save" on a button and in a price) stays in the source in the chunk and the patcher handles it (one recoverable #418). A string copied to a variable and later used as a lookup key (`labels[text]`) would be translated too: keep identifiers out of display strings, or mark the element `translate="no"`. Inline `<script>` code is not rewritten (only served chunks), and chunks over 4 MB are served untouched.
- **Worker mode** reads the language of a chunk from the `Referer`: with `Referrer-Policy: no-referrer` or `origin` on your site, chunks stay in the source language (patcher fallback). A client-side navigation between two languages inside the same Next.js app keeps the chunks already loaded: link your language switcher with a full page load (glot's switcher does). Subdomain mode has neither limit: each language is its own host.
- After the first deploy of bundle localization, a visitor whose browser already holds a source chunk marked `immutable` keeps it until your next deploy renames the chunk.

## 100 % JavaScript apps

Some apps arrive empty: the HTML is `<div id="root"></div>` (or an empty `<body>`) and everything is drawn by JavaScript or WebAssembly (Vite + React, Vue, Solid, Leptos or Yew in CSR mode, a Next.js app whose pages are client components). Search engines and link previews see nothing to translate. glot Edge then translates a **snapshot**: the page's DOM after a headless browser ran it, in the source language, translated at serve time with the same translation memory version as everything else. With `rendering: auto` (the default) glot uses a snapshot only when the origin's HTML looks empty; `prerender` always uses one; `origin` never does.

There are two ways to produce snapshots. Pick one; both can coexist (imported snapshots win).

### Option 1: rendered in your CI (no browser on the glot server)

Your build already has the app running for its tests: render the public routes there and upload them.

```bash
npx playwright install --with-deps --only-shell chromium   # once per CI image: the headless shell only
npx vite preview --port 4173 &                             # or `npx serve dist`, `next start`…
# GLOT_TOKEN: an API token from the dashboard, stored as a CI secret
glot prerender --base-url http://localhost:4173 --routes /,/pricing,/features --upload
```

- `glot prerender` writes `<route>/index.html` and `manifest.json` into `--out` (default `.glot/prerender`), then `--upload` posts them to `POST /edge/v1/{project}/snapshots` with an API token of a project admin. The project and server come from `[server]` in `glot.toml`, or `--server` and `--project` if you have no `glot.toml` (glot Edge needs no keys).
- A snapshot is stored under the page's path on your origin (`/pricing`; the query string is ignored) and replaced by the next upload, so run it on every deploy.
- `--append` for apps that append to `<body>` instead of filling a root element (Leptos `mount_to_body`, Svelte `target: document.body`): the snapshot is wrapped so the app does not render twice.
- Limits: 100 snapshots per request (the CLI batches), 5 MiB per page, 2,000 paths per project.

### Option 2: the renderer service (self-hosting)

`packages/renderer` is a small service that renders pages in the background and keeps the snapshots on disk. It is off by default in `deploy/compose.prod.yml`; turn it on with a token in `deploy/.env`:

```bash
echo "GLOT_RENDERER_TOKEN=$(openssl rand -hex 32)" >> deploy/.env
docker compose --profile renderer up -d
```

It is sized for a 2 GB server:

- **One browser, one page at a time.** `chromium-headless-shell` (not full Chrome) in a `node:slim` image, started on demand, stopped after `GLOT_RENDERER_IDLE_S` (60 s) without work, restarted every `GLOT_RENDERER_RECYCLE_AFTER` (50) renders or when the container passes `GLOT_RENDERER_RECYCLE_MB` (320 MB).
- **Bounded.** `mem_limit: 450m`, `cpus: 1.0`; images, video, fonts, WebSockets and analytics, ads or chat scripts are never fetched; a page has `GLOT_RENDERER_TIMEOUT_MS` (15 s) and a hung page kills the browser; the queue holds `GLOT_RENDERER_MAX_QUEUE` (100) pages, beyond which glot simply serves the origin.
- **Never in a visitor's request.** The first visit to a page serves the origin HTML and queues a render; later visits get the snapshot. After `GLOT_RENDERER_TTL_S` (24 h) the stored snapshot is still served while a fresh one renders in the background.

Measured with `docker stats` (1 vCPU, 450 MB limit), rendering a Vite + React SPA, a Next.js static export and a Leptos/WebAssembly app (ClipOnIt), 30 pages in a row: **298 MB peak**, about 90 MB idle with the browser stopped, median render 1.5 s, p95 3.1 s, no failures.

### What a snapshot does not contain

- Pages behind a sign-in: the browser has no cookies. Keep those on the origin ([application pages](#application-pages-sign-in-oauth-api)).
- Live data pushed over WebSockets or server-sent events, and content that waits for images or fonts to load.
- Text your app renders after an interaction (a menu opened on click): the [patcher](#single-page-apps-and-hydration) translates it in the browser.

## Compared with other approaches

**Key-based i18n** (next-intl, i18next, glot catalogs): precise, reviewable, works offline, and remains the best choice for app UI. It needs keys in the code and a build or CDN per language; on a site that was never internationalized, that's a rewrite. glot Edge covers the text without keys today, and uses your keys where you add them.

**Translation proxies** (the "translate my site by DNS" products): no code changes, like glot Edge. They usually translate without your glossary or validation, give little control over what gets sent, often break single-page apps at hydration, and treat SEO as a checkbox. glot Edge validates every translation with your project's rules, never sends protected areas or personal data, publishes nothing new on sensitive pages without a review, measures coverage on the pages that matter, and reports what is indexable instead of claiming it.

What glot Edge does not do: translate images or text inside canvas or video, or know whether Google actually indexed a page without Search Console.
