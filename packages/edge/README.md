# @glot-i18n/edge

Locale URLs for glot on-demand locales, at the edge — and **glot Edge**: your rendered HTML translated by
segments from the project's translation memory, no i18n keys needed, indexable from the first byte.

## glot Edge (Localisation SEO)

`/es/pricing` = your `/pricing`, fetched and translated before it reaches the visitor (or Google): text,
`title`/`meta`/`alt`/`aria-*`, framework payloads (`__NEXT_DATA__`, RSC flight, `__NUXT_DATA__`), `<html lang dir>`,
canonical + reciprocal hreflang + x-default, internal links under `/es`. Unknown segments are reported (path only,
never query strings, cookies or personal data, never for logged-in requests), translated by glot with your
glossary and validators, and served from a following request; meanwhile the source text is served.

- **Fail-open**: glot slow or down → your page untranslated (`x-glot-edge: fail-open`), never an error. A cold
  start waits at most `timeout` ms (300) for glot; the last known TM is kept in memory and in `store`.
- **One version per page**: each request uses one TM snapshot (`x-glot-version: es@12`); a change in glot bumps
  the version, which is the page cache key (the purge).
- **Kill switch**: Edge off for the project or a domain in glot → passthrough within a minute, no redeploy.
- **Headers kept** (CSP, HSTS, cookies…); `Server-Timing: glot;dur=…, origin;dur=…` shows glot's cost.
- **SEO** (same output as the glot server's Rust rewrite, tested byte for byte): one canonical (the origin's,
  localized), hreflang for the **ready** locales only (`/cdn/{project}/locales.json`) + x-default, `og:locale`,
  `og:url`, JSON-LD texts through the TM and its site URLs localized.
- **Client patcher**, served same-origin (route `/__glot/*` to the middleware): `/__glot/patcher.js` (6.4 kB gzip,
  `async`), `/__glot/tm.json?locale=es` (the TM snapshot, `&since=12` → 204 until a newer one) and, with a `key`,
  `POST /__glot/missing`: text that only exists after the app's JavaScript ran is reported there (debounced, once
  per page view, never with a session cookie — HttpOnly ones are checked here — never personal data), forwarded to
  glot with the key, and applied in place when the new TM version lands.
- `translate` (the default `Translator`) is synchronous: `(html, tm, ctx) => { html, total, translated, missing }`.

**Cloudflare Worker** (`dist/worker.js`): on by default; `[vars]` as below plus `GLOT_KEY = "pk_…"` (discovery and
plan usage), `GLOT_TIMEOUT_MS`, `GLOT_EDGE = "0"` to keep only the `data-glot` catalog fill. Cache API = page cache
and last-known state.

**Next.js**: `glotProxy({ source: "en", locales: ["es"], edge: { server, project, key } })`, matcher including
`"/__glot/:path*"`.

**Express / Connect / node:http**:

```ts
import { glotEdge, fileStore } from "@glot-i18n/edge/express";
app.use(glotEdge({ source: "en", locales: ["es"], server: "https://api.glotlang.com", project: "site",
                   key: "pk_…", store: fileStore(".glot-edge") }));
```

**Subdomain mode** (`es.example.com`, nothing to install): CNAME to `edge.<glot domain>`, verify it in glot; the glot
server proxies and translates (same rules).

**Anything else**: `import { createEdge } from "@glot-i18n/edge/runtime"`.

```sh
npm i @glot-i18n/edge
```

**Next.js** (`proxy.ts`): `/de/x` renders `/x` with the header `x-glot-locale: de`.

```ts
import { glotProxy } from "@glot-i18n/edge/next";
export const proxy = glotProxy({ source: "en", locales: ["de", "it"] });
export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
```

**Cloudflare Worker** in front of a static site: `/de/pricing` = the origin's `/pricing` with every
`data-glot="key"` element filled from glot's `de` catalog.

```toml
# wrangler.toml
main = "node_modules/@glot-i18n/edge/dist/worker.js"
[vars]
GLOT_SERVER = "https://glot.example.com"
GLOT_PROJECT = "site"
LOCALES = "de,it,ja"
SOURCE_LOCALE = "en"
```

`import { route } from "@glot-i18n/edge"` is the routing rule both use.

## Copy experiments (segmented, decided at the edge)

The arm is chosen before rendering, so the first HTML already carries the variant: no flicker, and glot is
never on the critical path (config cached in memory, revalidated in the background; cold start waits ≤ 150 ms,
then serves the official text).

**Next.js 16** (`proxy.ts`, Node or Vercel Edge):

```ts
export const proxy = glotProxy({
  source: "en", locales: ["de", "it"],
  experiments: { server: "https://glot.example.com", project: "site", key: "pk_…",
    consent: (req) => req.headers.get("cookie")?.includes("cmp=all") ?? false, // optional
    attrs: (req) => ({ plan: "pro" }) },                                    // optional, never sent to glot
});
```
The app receives `x-glot-xp` (base64url JSON `{v, locale, a, o, k?}`, `k` = keys of each counted experiment, for the "seen" exposure); `withExperiments(messages)` from
`@glot-i18n/runtime` merges it in `getRequestConfig`. Varying responses get `Cache-Control: private, no-store`.

**Cloudflare Worker**: add `GLOT_KEY = "pk_…"` to `[vars]`; `data-glot` elements get the variant text (source
pages included) and `x-glot-xp` is forwarded to the origin.

**Any edge runtime** (Vercel Edge Middleware, Deno, Fastly…):

```ts
import { glotExperiments, forwardHeaders } from "@glot-i18n/edge/experiments";
const xp = glotExperiments({ server, project, key });
const d = await xp(request, locale, waitUntil);   // d.header, d.overlay, d.cookies
fetch(origin, { headers: forwardHeaders(request.headers, d) });
```

- **Segment** from `cf-ipcountry` / `x-vercel-ip-country` / `cloudfront-viewer-country` / `fastly-geo-country`,
  served locale, device (`sec-ch-ua-mobile`, else UA), source (UTM, click ids, referrer), `visitor` (only with consent).
- **Arm**: weights of the matched cell; random per document, or `decide(req, config, { unit })` hashes a unit you own.
- **Persistence** (project setting): `none` = no cookie; `session` = `glot_xp=<id>.<arm>.<cell>~…`, no identifier,
  no expiry. With a consent signal only: `glot_seen` (1 year) for new/returning.
- **Not counted**: bots (served exactly like everyone, no cloaking), prefetches, GPC/DNT (official text),
  `?glot_xp=<id>.<arm>` previews, personalized cells. Exposures are batched to `/api/insights/{key}/batch` in `waitUntil`.
- **Fallback**: no config, glot down past `max_stale`, malformed config → official text, nothing counted.
- `decide()` ≈ 15 µs (5 experiments + holdout), ~3.7 kB gzip.

MIT — part of [glot](https://github.com/MattJeff/glot).
