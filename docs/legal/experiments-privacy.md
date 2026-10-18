# Segmented experiments and privacy — notes for site owners and their DPO

> **Not legal advice.** This page describes what glot technically does so that the site owner (the data controller)
> and their DPO or counsel can assess it. glot does not conclude that any configuration is compliant or exempt from
> consent: that assessment depends on the site, its other trackers, its notice and the applicable national law, and it
> belongs to the controller. Status of the texts cited: as known on 2026-10-18; check for updates.

## 1. What is processed, where

| Step | Where | Data | Kept by glot |
|---|---|---|---|
| Visitor context (country, region, device, source, locale, app attributes) | the site's edge (Next proxy / Worker), in memory, per request | CDN geo headers (derived from the IP by the CDN), client hints / user agent, referrer, UTM | nothing: only the name of the matched cell leaves the edge |
| Arm decision | the site's edge | random draw + public configuration | nothing |
| Exposure / conversion | `POST /api/insights/{public key}` | `{experiment, cell, arm}` list, event name, optional revenue value and currency | daily aggregate counters per `(experiment, cell, arm, day, event)` |
| Persistence of the arm | the visitor's browser | see §2 | — |

glot stores **no IP address, user agent, route, visitor or session identifier**. App attributes (`plan`, `custom.*`) are
restricted to short declared values and never leave the edge. Bots (by user agent), opted-out visitors, Global Privacy
Control and Do Not Track are served the official text and are **not counted**. glot's server logs follow the
self-hosting configuration of the operator (glot cloud: see its own privacy policy).

The IP address is still processed by the site's CDN to derive the country: that is the CDN's and the site's processing,
outside glot, and should be covered by the site's existing notice.

## 2. Persistence modes

Set per project (`PUT /api/projects/{slug}/xp/settings {"persistence": …}`).

### `none` (default)
- No cookie. The edge draws an arm for each HTML document. In the browser, `@glot/runtime` / `@glot/insights` keep the
  assignment in **`sessionStorage`** (`glot_xp`, `glot_xp_sent`, `glot:insights`) so conversions on later pages are credited to the arm actually seen.
- Limit: the edge cannot read `sessionStorage`, so a reload or a second page may show another arm (diluted effect).
  Suited to single landing pages, not to multi-page funnels.
- **Note:** ePrivacy art. 5(3) (France: art. 82 of the *loi Informatique et Libertés*) applies to *any* storage of or
  access to information on the terminal, not only cookies; `sessionStorage` is in scope (see EDPB Guidelines 2/2023 on
  the technical scope of art. 5(3)). "No cookie" therefore does not by itself mean "no consent question".

### `session` (recommended for server-rendered sites)
- One first-party **session cookie** `glot_xp` (no `Expires`/`Max-Age`: deleted when the browser session ends),
  `Path=/`, `SameSite=Lax`, `Secure`, readable by the page script. Content: only `experiment-id.arm.cell` triples
  (holdout included). No identifier, nothing unique to the visitor: many visitors carry the same value.
- Used solely to keep the same text across the pages of one visit and to attribute conversions to it, for the
  site owner's own aggregate statistics.

### Attributes that need consent
`visitor = new / returning` requires remembering the visitor across sessions. The edge only computes it when the
site's `consent(req)` callback (passed to `glotProxy`) returns true; it then sets a first-party cookie `glot_seen=1`
(`Max-Age` 1 year, no identifier). Without consent, `visitor` is `unknown` and segments using it never match.

## 3. The CNIL "audience measurement" exemption — points to check

The CNIL (guidelines and recommendation on cookies of 17 September 2020, and its pages on audience measurement tools)
accepts that some audience-measurement trackers can be exempt from consent when they are **strictly necessary** to the
service as the publisher operates it. The conditions it lists include, in substance:

1. a purpose **strictly limited** to measuring the audience / performance of the site, for the **publisher only**;
2. production of **anonymous statistics** only; no combination with other processing, no transfer of the data to third
   parties, no cross-site tracking or tracking of the visitor's navigation on other sites;
3. trackers lifetime limited (the CNIL indicates 13 months, not automatically extended) and retention of the collected
   information limited (25 months);
4. information of users and an easy way to object.

How glot's design maps to these points — **for the controller to assess, not a conclusion**:

| Point | What glot does | Open question for the DPO |
|---|---|---|
| Purpose | measure which text performs best on the site, aggregate counters, publisher's dashboard only | Is *A/B testing of copy* within "audience measurement / performance of the site" in the controller's reading? The CNIL's examples include performance measurement and ergonomics optimisation; it has not, to our knowledge, ruled specifically on copy experiments. **Segment-specific personalisation** ("promote for this segment") changes what a visitor sees based on their context: assess it separately; it is likely *not* audience measurement. |
| Anonymous statistics | daily counters per cell and arm; no identifier, no IP, no UA stored | Small cells (low traffic segments) could make counts very small; glot does not apply a k-anonymity threshold. |
| No cross-site, no third party | first-party cookie / storage, glot as processor of the site owner | If glot cloud is used, a processor agreement (art. 28 GDPR) is needed; check sub-processors and hosting location. |
| Lifetime | `session` cookie and `sessionStorage` end with the browser session | `glot_seen` (consent only) lasts 1 year: within the 13-month bound, but set only after consent. |
| Retention | counters kept while the experiment exists; deleted with it or with the project | Set a retention period (≤ 25 months) for stopped experiments in the controller's policy. |
| Information & objection | GPC (`Sec-GPC`) and DNT honoured at the edge | The site's notice must mention the experiments and the opt-out; provide a visible opt-out link. |

Revenue values (`track("purchase", { value })`) are aggregated (sum and sum of squares per arm) and not linked to
an order or a person; still list them in the notice.

When in doubt, the safe configuration is: `persistence: "none"` **behind the consent banner** (load the beacon and
enable experiments only after consent), and the official text for everyone else.

## 4. Consent integration

- Before consent (or after refusal), if the controller chooses the consent route: do not pass the `experiments`
  option to `glotProxy` for that request and do not call `init` from `@glot/insights` — visitors get the official
  catalog, nothing is counted, nothing is stored by glot.
- The `consent(req)` callback of `glotProxy` (reading the CMP's cookie) enables the attributes of §2 that need it.
- GPC (`Sec-GPC: 1` / `navigator.globalPrivacyControl`) and DNT are treated as an objection at the edge and in
  `track()`, whatever the banner says.
- Note: there is no per-visitor opt-out switch in the libraries yet (only GPC/DNT); a site offering an opt-out link
  must implement it as "no `experiments` option for this visitor" (for example from a first-party preference cookie).

## 5. Digital Fairness Act and consumer law

The European Commission's **Digital Fairness Act** initiative (fitness check of EU consumer law, public consultation
2025; legislative proposal announced for 2026) targets, among other things, **dark patterns**, **unfair
personalisation** (in particular personalisation exploiting vulnerabilities or personal data in a way that harms
consumers) and manipulative marketing. Its final content and timing are not settled; check the current status.

Independently of the DFA, the Unfair Commercial Practices Directive (2005/29/EC) and, for online platforms, art. 25 of
the Digital Services Act already apply. What glot does to limit risk — **the site owner remains responsible for its
texts**:

- generated arms may not add claims, prices or urgency; numbers, percentages, currencies and dates must come from the
  source or from facts the owner declared, otherwise the arm is rejected;
- keys tagged `pricing` or `legal` cannot be in an experiment (no price personalisation);
- segments use coarse context (country, device, source, declared plan), never sensitive data or inferred
  vulnerabilities; `custom.*` values are limited to short declared enumerations;
- every arm says the same thing in substance (also required to avoid SEO cloaking).

Recommended in the owner's own process: review generated arms before activation (`review: true`), avoid fear- or
scarcity-based intentions, and document why each segment is targeted.

## 6. Checklist for the controller

- [ ] Choose `persistence` (`none` / `session`) and document the reason.
- [ ] Decide, with the DPO, whether experiments are run under the audience-measurement exemption or under consent.
- [ ] Update the privacy notice / cookie table (`glot_xp`, `glot_seen`, sessionStorage keys) and add an opt-out link.
- [ ] Sign a processor agreement if glot cloud is used.
- [ ] Set a retention period for stopped experiments.
- [ ] Review arms and segments for unfair practices; keep `pricing`/`legal` keys out.
