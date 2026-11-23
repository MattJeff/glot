# Cross-site priors: what converts in each language, learned from every glot site

## Problem

glot already tests translations on conversion (`variants.rs`: 2-3 validated variants of one key, Thompson sampling,
P(best) > 95 % before a promotion is proposed). Nobody uses it: `variant_experiments` has **0 rows** in production.

The reason is arithmetic, not UX. A small site never has the traffic to finish a test in one language:

- cliponit converts 302 signups out of 39 476 views in 7 days (0.77 %). Detecting a +20 % relative lift at that rate
  needs roughly 50 000 views **per arm** (≈ 16·p(1−p)/Δ²). In its biggest language that is months; in Bengali, never.
- Wanted Clips: 11 770 views in 7 days across 20 languages, so a few hundred per language per week.

Every arm starts from a uniform prior, Beta(1, 1) (`variants.rs`, `alpha: 1.0 + c`). Each experiment relearns, from
nothing, things another glot site already measured last month.

## The idea

Replace the uniform prior with a **prior pooled across every opted-in glot site**, per language and per kind of text
(primary CTA, pricing headline, free-trial line, form label…). A Hindi CTA test on a 300-view site then starts from what
Hindi CTAs did on every other site, and concludes with far fewer visits. The dataset this produces is the moat:

- "In Hindi, 'मुफ़्त में शुरू करो' (informal) beats 'निःशुल्क आरंभ करें' (formal) on signup CTAs, 41 sites, +18 % [9, 27]."
- "In ja, polite form beats plain form on pricing pages; the reverse on gaming sites."
- "pt-BR: 'sem cartão de crédito' outsells 'grátis' on SaaS trial buttons."

These are measured on real conversions, not guessed by an LLM. No single customer can produce them, and no competitor
can rebuild them without both serving the text and measuring the outcome on many sites.

### Why it does not exist yet

- Translation tools (Weglot, Lokalise, Phrase, Crowdin, Smartling) serve or manage text but do not see conversions.
- Analytics and A/B tools (GA4, Optimizely, VWO) see conversions but do not own the translated text, and run per site.
- glot does both: Edge / the website tag serve each segment, and Insights attributes the goal event to the arm.
  The missing piece is the pooling, plus a shared vocabulary for "the same kind of text" across sites.

## Design

### 1. Feature tags on variants (no text shared)

Each arm gets a small set of **features**, computed at generation time, that describe *how* it is phrased, never what
it says: `register: formal|informal`, `price_word: free|no_card|trial`, `length_bucket`, `imperative`, `urgency`,
`script_mix` (Hinglish), `emoji`. Plus the slot `kind` (cta_signup, cta_buy, headline_pricing, trial_line…) and the
site `vertical` (saas, gaming, travel…), declared by the customer.

### 2. Shared statistics: counts only

Per (locale, kind, vertical, feature value): pooled views and conversions, summed from experiments of opted-in
projects. Stored in a new table `prior_stats` with no project id, no text, no URL. A cell is published only when it
aggregates **≥ 5 distinct projects** (k-anonymity); otherwise it does not exist.

### 3. Hierarchical prior

For a new arm, its prior is Beta(m·κ, (1−m)·κ), where m is the pooled rate for its features (relative to the
control's) and κ the prior strength, estimated from between-site variance (empirical Bayes) and capped (κ ≤ 2 000
pseudo-views) so a site's own data overrides the crowd within a few thousand views. `thompson`, `posterior` and
`decide` keep working unchanged: only the starting alpha/beta change. The alpha-spending rule and minimum n per arm
stay, so false positives remain controlled (extend the existing `simulation_right_winner_and_controlled_false_positives`
test with informative priors, including a deliberately wrong prior).

### 4. Variant generation informed by the priors

`experiments/generate.rs` gets the pooled findings for the locale and kind as constraints ("in hi on gaming sites,
informal register and Hinglish lead"), so the arms tested are the ones most likely to win, not random paraphrases.
For projects that refuse LLM calls (Wanted: the founder writes copy by hand), the same findings are shown as a brief
and the human writes the variants.

### 5. Product surface

- Experiments page: "Starts from 41 sites in Hindi; expected to conclude in ~3 weeks instead of ~2 years."
- A public, aggregated "What converts in each language" report: marketing for glot, and only cells ≥ 5 projects.

## Privacy and consent

- Opt-in per project (off by default), revocable; revoking removes the project's contribution at the next rebuild.
- What leaves the project: counts per feature cell. Never text, URLs, visitor ids or the project's identity.
- Insights already stores only aggregate counters (no visitor id); this adds nothing about visitors.

## Prerequisites found while writing this

1. **Wanted sends no conversion event to Insights.** 11 770 `$view` in 7 days, zero signup or purchase events
   (cliponit sends `signup`, Orizn `signup_completed`). Without a goal event no test can conclude. First step:
   send `signup` and `purchase` (with `value`) from gta6clipmaker.com.
2. **No experiment has ever run.** Run one by hand on Wanted's main CTA in its 3-5 biggest languages to validate
   attribution end to end before building the pooling.

## Plan

1. Wanted: goal events in Insights; one hand-written CTA experiment (no LLM, per the founder's rule).
2. Feature tags on arms, `kind` on experiments, `vertical` in project settings.
3. `prior_stats` rebuild job (nightly) + k-anonymity floor + opt-in setting.
4. Informative priors in `variants.rs`, with the simulation test extended.
5. Prior-informed variant generation, and the brief for hand-written variants.
6. The public aggregated report, once enough cells pass the floor.

## Risks

- **Volume.** Pooling pays once there are tens of opted-in sites per language. With 3 sites it is a per-site bandit with
  extra steps: until then, value comes from step 1 and from the hand-written briefs.
- **Wrong priors.** A vertical whose audience differs could be pulled toward the wrong arm; the κ cap and a site's own
  data bound the damage, and the simulation test must cover it.
- **Feature drift.** Tags computed by an LLM can be inconsistent; keep them few, deterministic where possible
  (length, emoji, script mix), and versioned.
