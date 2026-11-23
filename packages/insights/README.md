# @glot-i18n/insights

Cookieless locale-demand and conversion beacon for glot Insights (< 2 kB): which languages your visitors
want, which ones convert, and language / copy experiments. The server only stores daily counters.

```sh
npm i @glot-i18n/insights
```

```ts
import { glot } from "@glot-i18n/insights";
await glot.init({ key: "pk_…", endpoint: "https://glot.example.com", onLocale: (l) => switchLocale(l) });
glot.track("signup");
glot.track("purchase", { value: 49, currency: "EUR" }); // revenue (0..1e6, ISO 4217)
```

Segmented copy experiments: `track()` credits the experiments / segment cell / arm this session was exposed to —
read from the `glot_xp` cookie (edge persistence `session`) and from sessionStorage `glot_xp` (copied from the edge's
`x-glot-xp` header by `<GlotExperiments>` of `@glot-i18n/runtime`). Each experiment is credited once per session and
event; nothing is attributed under Global Privacy Control / Do Not Track. Exposures themselves are counted by the edge;
"seen" exposures (`glot:seen` from the runtime) go to `/api/insights/{pk}/batch`, once per session and experiment.

Language launches (holdout): `onLocale` / the `glot:locale` window event give the locale to render, the new one or
the fallback for held-out sessions. An explicit choice wins and leaves the measurement: `explicit: true` (or
`data-explicit="1"`), a pick stored by `<glot-locale-switcher>`, or a `glot:locale-change` event mid-session.

Server side (no client JS): `import { reportVisit } from "@glot-i18n/insights/server"`.

Server-side conversions (webhook, API route): forward the visitor's `glot_xp` cookie value as is; the glot Edge proxy
sets it on your parent domain, so your server receives it. `served` = the locale the visitor browsed, `dedup` = a
stable id (order id) so a retried webhook counts once:

```sh
curl -X POST https://glot.example.com/api/insights/pk_… -H 'content-type: application/json' \
  -d '{"served":"es","event":"signup","glot_xp":"<cookie value>","dedup":"order_123456"}'
```

MIT — part of [glot](https://github.com/MattJeff/glot).
