# Insights

Which language should you add next, and did the last one pay off? Insights answers with your own traffic: demand per language, conversion per served locale, and language experiments with a confidence interval. It requires the [self-hosted server](self-hosting.md).

## Privacy

No cookie, no visitor id, no IP, user agent or route stored. The server keeps daily counters only: `(day, preferred language, served locale, country, experiment arm, event, count)`. One `sessionStorage` entry holds the experiment arm. A visit is counted once per browser session.

## Install the beacon

Get the project's public key in the dashboard (**Insights**), then either:

```html
<!-- dist/insights.global.js from @glot-i18n/insights, served by your app -->
<script src="/insights.global.js" data-key="PUBLIC_KEY" data-endpoint="https://glot.example.com" defer></script>
```

or, with a bundler:

```ts
import { init, track } from "@glot-i18n/insights";

init({ key: "PUBLIC_KEY", endpoint: "https://glot.example.com", locale: "fr", onLocale: (l) => switchTo(l) });
track("signup");    // a conversion event
```

The package is under 2 kB. Server side (Next middleware, Edge), report visits without any client script:

```ts
import { reportVisit } from "@glot-i18n/insights/server";

await reportVisit(request, { key: "PUBLIC_KEY", endpoint: "https://glot.example.com", served: "en" });
```

## What the dashboard shows

`/app/<project>/insights`:

- **Demand**: visits per preferred language (first valid entry of `navigator.languages`, else `Accept-Language`), whether you offer it, top countries, and for languages you do not offer, an estimate of extra conversions per 30 days and of the cost to translate all keys (dry-run estimate).
- **Funnel**: visits, conversions and rate per served locale, with a 95 % Wilson interval.
- **Experiments**: serve a new language to a share of its speakers (1-100 %) for 1-90 days. The verdict (`keep`, `remove`, `inconclusive`) waits for the required sample size (powered for a +20 % relative lift, α 0.05, power 0.8) or the end date, and shows the 95 % interval of the difference. Keeping a language adds it to your targets; removing it takes it out.

## Limits

The public endpoint `POST /api/insights/{public_key}` is rate-limited per project and per minute across replicas (default 1200/min, `GLOT_INSIGHTS_RATE_PER_MIN` or a per-project setting). Requests over budget get `429` with `Retry-After`. The key can be rotated from the dashboard; the old one then returns 404.
