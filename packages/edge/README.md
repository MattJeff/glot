# @glot-i18n/edge

Locale URLs for glot on-demand locales, at the edge.

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
