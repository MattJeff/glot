# Autopilot: sell revenue per market, not translated strings

## Problem

Every competitor answers "how do I translate this?". The customer's real questions are "which country do I open next, what is it worth, did it pay off, and is my French headline the one that sells?". Today they answer them with a spreadsheet, GA4 and a guess.

## What exists

On the market (searched November 2026):

- **Choosing a language** is sold as advice or as a report. Vendors publish "how to pick your next language" guides that say to look at browser languages in your analytics ([Localize](https://localizejs.com/articles/how-to-choose-localization-markets), [Localazy](https://localazy.com/blog/how-to-choose-the-right-languages-to-translate-to-when-expanding-globally/), [TransPerfect](https://www.transperfect.com/ja/node/7365)). CSA Research sells a [Global Revenue Forecaster](https://csa-research.com/business-analytics/global-revenue-forecast) built on market data, not on the customer's own visitors.
- **Translation tools show usage, not value.** Weglot reports visitors' browser languages and translation requests per language ([feature summary](https://partnersupport.apptegy.com/en/articles/14136063-key-features-of-weglot), [their GA guide](https://weglot.com/blog/language-google-analytics/)); Linguise and MultiLipi show page views per language. I found no product that turns this into an expected gain with an interval, a cost, and a button.
- **Testing translations on conversion** exists as a niche: SeaText markets AI variants and bandits per locale ([guide](https://seatext.seatext.com/guides/can-ai-help-generate-or-optimize-variants-for-a-b-testing-translated-content-6a8501ec)); otherwise the advice is "use Optimizely or VWO per locale". I found no evidence that Lokalise, Phrase, Crowdin or Smartling ship a per-locale bandit; the search was not exhaustive.

In glot before this work (audited, reused, not rebuilt):

| Need | Already there |
| --- | --- |
| Demand per language, conversion per served locale | `insights.rs`, `insight_daily` counters |
| Expected extra conversions per missing language, 90 % interval, confidence, cost | `recommend.rs` |
| Holdout / before-after measurement with honest verdicts, SRM, guardrails | `launches.rs` |
| Validated LLM variants served as a bandit, alpha-spending winner rule, promotion | `variants.rs`, `@glot-i18n/insights` |
| "Add this language and translate it" button | `next-language.tsx` (client side: two API calls, no measurement) |

So most of the statistics existed. What did not:

## What is new

1. **Money.** A value per conversion (declared, or measured from the `value` already sent with events) turns the recommender's conversions into currency, with the formula shown and a payback time.
2. **One click, server side, with the measurement scheduled.** Locale + capped job + launch measurement started when the translation is really done, each failure explained. Prediction frozen next to the measured result.
3. **Misrouted visitors.** A language you already ship whose visitors are served another one. No competitor material mentions it; on real data it is the largest finding (see the report).
4. **The optimiser loop closed.** Which texts matter (and never legal copy), a legal-safe filter on generated wordings, automatic withdrawal of a harmful wording, auto-keep or human approval, automatic stop on "no difference", and a report with interval and money.
5. **One page** that ranks, explains and acts.

## Architecture

One module, `crates/server/src/autopilot.rs`, one migration (`0183_autopilot.sql`), one worker tick (same period as the launch worker).

```
insight_daily ──► recommend::for_project ──► autopilot::rank (money, payback, missing data)
                                         └─► autopilot::misrouted
POST /autopilot/open ─► targets += locale ─► jobs (translate, capped) ─► autopilot_openings(translating)
        tick: job done & ≥ 90 % translated ─► launches::create ─► autopilot_openings(measuring) ─► launches worker
POST /autopilot/optimizations ─► variants::create ─► legal_safe filter ─► autopilot_optimizations(running)
        serving & winner rule: variants.rs (unchanged)
        tick: harmful() → delete arm | decided → promote (auto) / await approval | aged → stop
```

Existing handlers are called as functions (`launches::create`, `variants::create/promote/list`), so validation, plan checks and write paths stay in one place. Three one-line hooks in shared files: `lib.rs` (module + routes), `main.rs` (worker), `insights.rs` (`autopilot::on_revenue` in `ingest`).

Statistics:

- Opportunity gain: `recommend::gain` (difference of two Beta posteriors, 90 % interval), multiplied by the value.
- Harm rule: P(arm < official) > 99.5 % at each look; looks when total views reach arms · max(min_n/2, 25) · 2^look. A fixed bar on purpose: a false withdrawal costs one variant, a late one costs sales.
- Lift report: 20 000 seeded draws of the ratio of the two posteriors; median and 2.5 / 97.5 percentiles.

## Data model

- `autopilot_settings(project_id, goal, value_per_conversion, currency, mode)`
- `autopilot_revenue_daily(project_id, day, event, currency, n, value)`
- `autopilot_openings(id, project_id, locale, mode, days, goal, status, job_id, launch_id, estimate, error, created_by)`
- `autopilot_optimizations(experiment_id, project_id, kind, mode, status, reason, max_days, harm_looks, dropped, result, created_by)`

## Privacy and security

- New collection: a count and a sum per (day, event, currency). Nothing else. Not split by language, country or arm.
- Nothing personal goes to the LLM: variant generation sends the key, source text, current translation and style guide (as `variants.rs` already did).
- No customer URL is fetched. Writes need the admin role and a verified e-mail; reads need membership.
- Autopilot never writes a translation without validation, never touches legal keys, and in the default mode never writes without a human.

## PostHog import (added after the first version)

Many customers already have months of analytics. `autopilot_posthog.rs` imports PostHog's history (read-only Query API, HogQL, scope `query:read`) as daily aggregates into `autopilot_posthog_daily`, which has the same shape as `insight_daily` and is kept apart so Insights and launch measurement only ever count the glot tag. The engine reads whichever source has more visits and labels every number with it (`recommend::for_table`). Person and session ids are counted inside PostHog (`uniq`, `count()` over a per-session subquery); only counts, a browser-language prefix, a project locale code and a country leave PostHog. Tested against a mock built from the official docs; not tested against a real account (no key).

## Pricing idea

Opportunities with demand and cost are free (they sell the upgrade). Expected gains, money, one-click open with measurement, and auto-keep belong to Growth (already how `recommend.rs` and `launches.rs` gate). Later: a success fee is tempting but needs causal attribution customers accept; start with the holdout result on the invoice as proof, not as a price.

## Phases

1. **Done**: everything above, tested.
2. Per-key exposure (aggregate `glot:seen` per key) to rank texts by real views; bounce proxy (a second event on engagement).
3. Post-promotion holdback (5 % stay on the old wording for two weeks) to remove the winner's curse from the reported lift.
4. Revenue as the optimised metric (value per visitor instead of conversion rate), using the Bayesian engine of segmented experiments.
5. Weekly e-mail: "German is now worth X; your French CTA earned Y".

## Risks

- **Thin data is the norm.** Most projects will see "too little data" for weeks. The page must stay useful then (demand, cost, what is missing), and never fill the gap with a guess.
- **The estimate is not causal.** Visitors of an unserved language differ from served ones. Mitigation: wording, interval, and the holdout that follows.
- **Winner's curse** on promoted wordings: stated, lower bound highlighted, phase 3 fixes it.
- **Low-traffic locales** never conclude; the candidate list says so before the test starts.
- **Static exchange rates** in payback (reused from experiments): fine for an order of magnitude, wrong for accounting.
