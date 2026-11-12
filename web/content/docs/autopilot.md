# Autopilot

Autopilot answers two questions with your own traffic: which market should you open next and what is it worth, and which wording of a headline or button earns more. It runs on the counters [Insights](insights.md) already keeps and requires the [self-hosted server](self-hosting.md) or glot cloud.

> **Experimental.** Autopilot never shows an invented number. With no traffic it shows an empty page; with thin traffic it says how many visits or conversions are still missing.

## What is collected

Counters only. No visitor id, cookie, IP address, user agent or page is stored.

| Table | Content |
| --- | --- |
| `insight_daily` (Insights) | day, preferred language, served locale, country, experiment arm, event, count |
| `variant_arms` (copy experiments) | experiment, wording, views, conversions |
| `autopilot_revenue_daily` (new) | day, event, currency, number of events, sum of their `value` |

The revenue table is filled only when you send a value with a conversion:

```ts
import { track } from "@glot-i18n/insights";
track("purchase", { value: 49, currency: "EUR" });
```

It is not split by language, country or arm. A day with a single valued event stores that one amount, still without anything that identifies who paid.

## What a conversion is worth

Money estimates need one figure: the value of a goal conversion.

- **Declared**: set it on the Autopilot page (value and currency). It wins.
- **Measured**: otherwise, the average `value` of the goal event over 90 days, once at least 10 valued events exist, in the currency most of them use.
- **Unknown**: otherwise every estimate stays in conversions. No money is shown.

## Markets to open

For each language your visitors prefer and you do not serve (or serve less than 90 % translated):

```
extra conversions per month = visits per month × (native rate − fallback rate)
money per month             = extra conversions × value per conversion
```

- **fallback rate**: goal conversions ÷ visits of these visitors today, served another language.
- **native rate**: the same ratio for visitors served their own language (other languages than the source when there are at least 100 such visits, the source included otherwise).
- Both rates are Beta posteriors with a uniform prior. The range shown is a 90 % interval of the difference. The value per conversion is taken as exact.
- Translation cost is the same dry-run estimate as `glot translate --dry-run`. Payback = cost ÷ daily expected gain, converted with static reference exchange rates.

Ranking: languages with enough data first, then by expected gain, then by demand, then the cheaper one. A language is "too little data" under 30 visits from its speakers or under 100 visits in the comparison group; the page then states how many are missing and shows no gain.

The estimate assumes these visitors would convert like the ones already served their language. Visitors of different languages differ in more than language, so read it as a reasoned guess, then measure.

### Open this market

One click does three things:

1. adds the language to the project targets;
2. queues a translation job for that language only, capped at the estimate + 50 % (minimum $0.50);
3. once the job is done and at least 90 % of the strings are translated, starts a [launch measurement](insights.md) with the goal from your settings.

Two measurement methods, chosen in the dialog:

- **Holdout** (default): 20 % of the visitors who prefer the language stay on the current one for 14 days. The difference between the two groups is the effect of the language. Needs translations loaded at runtime (glot CDN, glot Edge).
- **Before / after**: the same audience before and after. Seasonality and campaigns move the numbers too, so the result is labelled a correlation and never "positive".

The prediction shown when you clicked is frozen next to the measured result. If something fails (job failed, budget reached, translation incomplete, measurement refused), the card says what happened and what to do; the language stays in your targets with untranslated strings falling back to the source.

## Visitors served the wrong language

A language you already ship whose visitors see another one (at least 30 such visits and 5 % of its audience). Often the cheapest gain: nothing to translate, a language detection or switcher fix. The same formula compares the two groups; under 10 conversions in the thinner group the page says how many are missing instead of a gain. Some of these visitors chose the other language themselves, which the counters cannot tell outside a language experiment.

## Import your PostHog history

Already using PostHog? Autopilot can rank markets on its history instead of waiting for the glot tag. It is read-only and fetches daily counts only.

**What glot asks PostHog for**: through the [Query API](https://posthog.com/docs/api/queries) (`POST /api/projects/{id}/query/`, HogQL), per day, browser language (`$browser_language`, the first language of the browser), page language and country (`$geoip_country_code`):

- sessions, pageviews, sessions with a single page, distinct visitors;
- sessions that fired your conversion event (chosen from your project's real event names).

Person ids and session ids are counted inside PostHog and never returned. Paths and hosts only come back as a language code of your glot project (`/nl/…`, `nl.example.com`), or as the language you set for pages without a code. No URL, IP address, person or event payload is fetched or stored. The key is stored encrypted and never shown again.

**Connect**:

1. In PostHog, avatar menu → Settings → Personal API keys → Create personal API key. Scope: Query → Read (`query:read`), limited to the project. The project key (`phc_…`) of your snippet cannot read: glot refuses it and says so.
2. Copy the Project ID (Project settings → General).
3. In glot, Autopilot → Data source: PostHog. Region (US: `us.posthog.com`, EU: `eu.posthog.com`, or your instance URL), project ID, key. **Test connection** lists your events and how many pageviews carry a browser language.
4. Pick the conversion event and the language of pages without a language code, then **Save and import**: 90 days by default (7 to 365), in weekly queries. Then glot refreshes every 6 hours from the last imported day.

The page says which source each number comes from ("Source: PostHog · days of data: 41"). When both sources have visits, the busier one is used and a link switches to the other. Copy tests and launch measurements still need the glot tag: only the tag can serve a wording or hold visitors out.

Refusals say what to fix: a key from the wrong region (401), a key without the Query Read scope (403), an unknown project ID (404), PostHog's hourly query budget (429, retried at the next sync).

## Wordings that optimise themselves

Autopilot lists translated texts worth testing: headlines, buttons and pricing copy, recognised from the key name (`hero.title`, `pricing.cta`, `signupButton`…) and the extracted UI role, weighted by the traffic of the locale. Legal copy (`terms`, `privacy`, `consent`, `refund`, `policy`…) is never listed and is refused if asked.

Each candidate shows when a first result could come: the slower of 3 wordings × 500 visitors and about 30 conversions per wording, at the locale's current pace. A locale without any conversion yet says so, and its texts cannot be tested: the test could never conclude. Navigation, account, back-office and copy-to-clipboard buttons are left out.

**Optimise** starts a [copy experiment](insights.md) on that key and locale:

1. The model writes two alternative wordings of the official translation. Each must pass the usual validation (placeholders, ICU, glossary, brand terms).
2. **Legal-safe check**: a wording that adds a number, a price or percentage, a link, an e-mail address or a promise word ("free", "guaranteed", "best"…) the official translation does not contain is refused before any visitor sees it.
3. `@glot-i18n/insights` serves the wordings: an equal share until each has its minimum sample (500 visitors by default), then top-two Thompson sampling.
4. **Winner**: probability of being best above 1 − 5 % ÷ (arms × 2^(look+1)), checked only when traffic doubles. The 5 % error budget is split across arms and looks, so looking repeatedly does not manufacture winners.
5. **Harmful wording**: each time traffic doubles (from half the minimum sample), a wording whose probability of converting worse than the official translation exceeds 99.5 % is withdrawn and never served again. If every new wording is withdrawn, the test is rolled back. The bar is deliberately easier to reach than the winner's: withdrawing a harmless wording by mistake costs one variant, leaving a harmful one live costs sales.
6. **No difference**: after 30 days without a winner the test stops and the official translation stays.

When a wording wins, **approval** mode (default) waits for you; **auto** mode writes it as the official translation through the normal write path (validated, in the history, shipped by `glot pull`, the CDN or the GitHub App). If the official translation wins, nothing changes.

The report gives the relative lift with a 95 % credible interval and, when the value per conversion is known, what it is worth per month if the lift holds. The lift of a winner tends to be overestimated because it was picked while ahead; the lower bound is the safe figure.

## API

| Method | Path | |
| --- | --- | --- |
| `GET` | `/api/projects/{slug}/autopilot?days=30` | everything the page shows |
| `PUT` | `/api/projects/{slug}/autopilot/settings` | `{goal, value_per_conversion, currency, mode}` |
| `POST` | `/api/projects/{slug}/autopilot/open` | `{locale, mode?, days?}` |
| `POST` | `/api/projects/{slug}/autopilot/optimizations` | `{key, locale, mode?, min_n?, max_days?}` |
| `POST` | `/api/projects/{slug}/autopilot/optimizations/{id}/approve` | keep the winner |
| `POST` | `/api/projects/{slug}/autopilot/optimizations/{id}/stop` | stop, official translation stays |

## Limits

- No bounce rate: Insights does not record page exits, only visits and the events you send.
- Texts are ranked by locale traffic, not by how often each string is seen.
- One value per conversion for the whole project; no margin, no lifetime value.
- A promoted wording is not re-measured after promotion.
