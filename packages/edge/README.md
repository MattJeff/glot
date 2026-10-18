# @glot/edge

Locale URLs for glot on-demand locales, at the edge.

```sh
npm i @glot/edge
```

**Next.js** (`proxy.ts`): `/de/x` renders `/x` with the header `x-glot-locale: de`.

```ts
import { glotProxy } from "@glot/edge/next";
export const proxy = glotProxy({ source: "en", locales: ["de", "it"] });
export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
```

**Cloudflare Worker** in front of a static site: `/de/pricing` = the origin's `/pricing` with every
`data-glot="key"` element filled from glot's `de` catalog.

```toml
# wrangler.toml
main = "node_modules/@glot/edge/dist/worker.js"
[vars]
GLOT_SERVER = "https://glot.example.com"
GLOT_PROJECT = "site"
LOCALES = "de,it,ja"
SOURCE_LOCALE = "en"
```

`import { route } from "@glot/edge"` is the routing rule both use.

MIT — part of [glot](https://github.com/MattJeff/glot).
