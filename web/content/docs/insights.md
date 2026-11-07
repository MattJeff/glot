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

## Language recommendations

The **Next language** card (project overview and Insights) and `GET /api/projects/{slug}/insights/recommendations?days=30&goal=signup` rank the languages to add or to finish, from the counters above. Nothing extra is collected.

**Candidates**: a preferred language you don't offer, with visitors served another locale today (*missing*), or a target locale under 90 % translated (*under translated*: missing strings fall back to the source). The source language is never a candidate. Visits without a usable language (`und`) only count in the total.

**Per candidate** L, over the window of `days`:

- **Demand**: visits from L, scaled to 30 days (`monthly = visits × 30 / days`), their share of all visits, and the trend (second half of the window vs the first; flagged when it moves by 20 % or more with at least 10 visits in each half).
- **Current experience**: the locale L visitors are served most today and the share of their visits it covers.
- **Fallback rate**: goal conversions / visits of L visitors today. **Native rate**: the same ratio for visitors served their own language, other languages than L and the source. When that group has under 100 visits, source-language visitors are included (flagged: they may convert differently for reasons that have nothing to do with language).
- Each rate gets a Beta posterior with a uniform prior: `Beta(1 + conversions, 1 + visits − conversions)`. With little data the posterior stays wide instead of reading 0 % or 100 %.
- **Expected gain**: `monthly × (E[native] − E[fallback])` conversions per month, with a 90 % interval `monthly × (Δ ± 1.645 × √(Var[native] + Var[fallback]))` (normal approximation of the difference of the two posteriors). The interval can go below zero; when its top is ≤ 0 the card says there is no measurable gain.
- **Cost**: the `glot translate --dry-run` estimate (characters → tokens × the project model's price) for every key (*missing*) or the keys not yet translated (*under translated*). **Cost per expected conversion** = cost / expected monthly gain, i.e. the first-month payback, when the gain is positive.
- **Confidence**: *not enough data* under 30 visits from L or under 100 native visits; *solid* from 300 visits on both sides and 10 conversions each (from 300 visits of demand when conversions aren't tracked); *indicative* in between.

**Ranking**: expected gain (floored at 0), then demand, then cost. If the goal event never fires in the window, no gain is computed: the card says conversions aren't tracked and ranks on demand, then cost.

**Add {language}** adds the locale to the targets and starts a translation job for it with `budget_usd` set to the estimate + 20 % (jobs accept `budget_usd`; the stricter of it and the project budget wins, checked after each batch of keys).

Assumptions and limits:

- The gain assumes L visitors would convert like visitors already served their own language. That is an observational comparison, not a controlled one: to measure it, run a [language experiment](#what-the-dashboard-shows).
- Conversions are counted per `track()` call, visits once per browser session, so a visitor who signs up twice counts twice. Rates are capped at 1.
- Language is the first valid entry of `navigator.languages`, reduced to its primary subtag (`pt-BR` → `pt`); the suggested locale is the market default (`pt` → `pt-BR`).
- No revenue is used: payback is in conversions, not money.

## Measuring a language launch

**Insights → Launches → Measure a launch** records what adding a language did to the goal conversion of the visitors who prefer it (preferred language = the launched one). The locale joins the targets; translate it before traffic arrives. Each launch keeps: locale, date, method, the audience's baseline (visits and conversions in the N days before), the translation spend (jobs that named this locale, a multi-locale job prorated; the dry-run estimate when there is none) and glot's [recommendation](#language-recommendations) for that language at launch time, if it had one.

Two methods:

- **Holdout** (catalogs loaded at runtime: glot CDN, `<glot-locale-switcher>`, `glot.loadCatalog`). For N days (7-90, default 14), each session of a visitor who prefers the language is drawn once: 80 % get the new locale, 20 % (5-50 %, configurable) stay on the fallback (the source by default). The beacon tells your app which one: `onLocale(locale)` / the `glot:locale` window event carry the new locale *or* the fallback. The draw is per browser session (sessionStorage), so a page never switches language mid-session. After N days, or earlier on a decisive win, the holdout stops and everyone gets the language (**rolled out**).
- **Before / after** (catalogs compiled into the app, so nothing can be held out). The same audience in the N days before vs the days since the launch. **Not causal**: seasonality, campaigns and trends move conversions too. The page says so, and the verdicts never say "positive": only *higher after*, *lower after* or *no clear change*.

**Explicit choice always wins.** A visitor who picks a language (a pick in `<glot-locale-switcher>` is stored in `localStorage["glot-locale"]`; or pass `explicit: true` to `init`, `data-explicit="1"` on the script tag, `explicit` to `reportVisit`) is never assigned, so never held out. A pick mid-session (`glot:locale-change`, which the switcher dispatches; dispatch it from your own picker too) withdraws the session from its arm once (counted as `$explicit`) and later conversions carry no arm. Those sessions are excluded from the analysis and counted on the page. Caveat: if held-out visitors pick the language themselves much more often than the others leave it, the remaining groups differ slightly; compare the two exclusion counts.

**Statistics** (holdout): the engine of copy experiments, one cell and two arms. Conversion rate per arm has a Beta posterior; the result is the relative lift of the new locale over the fallback with its 95 % credible interval, and the probability it converts better. Guards: no verdict before 7 days and `min_n` (200) visitors per arm; interim decisions spend the 5 % error budget across looks (alpha spending), so checking every day does not inflate false wins; a χ² sample-ratio-mismatch test on the assignment counts (p < 0.001) marks the measurement **invalid** (something serves the wrong locale to part of the audience). Verdicts: *collecting*, *probably positive*, *probably negative*, *not conclusive yet*, *invalid*. The verdict is frozen when the measurement ends.

**Share** creates a read-only public page (`/case/{token}`, with an OG image) for a case study: the story (recommendation → cost → audience → result with its interval) and the daily curve, aggregates only. The token is an HMAC of the launch and a random nonce: **Revoke** kills the link at once, sharing again draws a new one. The page is not indexed by search engines and never cached.

## Limits

The public endpoint `POST /api/insights/{public_key}` is rate-limited per project and per minute across replicas (default 1200/min, `GLOT_INSIGHTS_RATE_PER_MIN` or a per-project setting). Requests over budget get `429` with `Retry-After`. The key can be rotated from the dashboard; the old one then returns 404.
