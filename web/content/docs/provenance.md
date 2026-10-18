# Translation provenance

`glot.lock` records how every translation was produced. glot can export that record and show it in your app. This page describes what glot records; it is not legal advice.

## Why it may matter

Article 50 of the EU AI Act (Regulation (EU) 2024/1689) sets transparency obligations for some AI systems and AI-generated or AI-manipulated content. They apply from 2 August 2026. Whether they cover the machine-translated UI of your product, and what disclosure is enough, depends on your situation and on guidance that is still being published (Commission guidelines, codes of practice). glot gives you the facts per string; your legal team decides what to do with them.

## What is recorded

For each locale and key, `glot.lock` keeps:

| Field | Meaning |
|---|---|
| `origin` | `machine` (with `model`), `human` (written or fixed with `glot set`, the overlay or the dashboard) or `imported` (existed before glot, author unknown) |
| `translated_at` | When the model produced the current text (UTC) |
| `reviewed`, `reviewed_at`, `reviewed_by` | Whether a human wrote or approved it, when, and who (`GLOT_REVIEWER` when set, e.g. `GLOT_REVIEWER="Ana" glot set fr home.cta "Commencer"`) |
| `score` | Judge score 0-100, when a judge model ran |

Timestamps are kept while the text is unchanged and refreshed when it changes. Entries written before this feature have no dates.

## Export

```sh
glot provenance export                  # JSON on stdout
glot provenance export --format csv -o provenance.csv
glot provenance export --format html -o provenance.html
```

Each row: `locale, key, origin, ai_generated, model, translated_at, reviewed, reviewed_at, reviewed_by, score`. `ai_generated` is true for machine translations no human has reviewed since. The export never includes the translated text, only metadata.

## In the app

```sh
glot provenance catalog   # writes messages/fr.provenance.json, messages/de.provenance.json…
```

Run it after `glot translate` (or in CI) and ship the files with your catalogs. Then, with `@glot-i18n/runtime`:

```ts
import { provenanceOf, provenanceLabel, hasMachineTranslations } from "@glot-i18n/runtime/provenance";
import prov from "../messages/fr.provenance.json";

// Page-level notice
if (hasMachineTranslations(prov)) showNotice("Cette page a été traduite avec l'aide de l'IA.");

// Per string, e.g. next to a legal text
provenanceLabel(provenanceOf(prov, "billing.terms"), {
  locale: "fr",
  labels: { ai: "Traduit par IA", reviewed: "relu le {date}" },
}); // "Traduit par IA · relu le 3 sept. 2026"
```

`provenanceLabel` returns `null` for human-written and imported strings. `provenanceBadge` returns the same label as a small `<small class="glot-provenance">` element. Translate the labels like any other UI string.

## Limits

- The record is as good as the workflow: an edit outside glot shows up as `human` on the next run, without a reviewer name.
- Entries pulled from the glot server (`glot pull`) carry origin and review status, but no dates yet.
- `imported` means unknown, not human: glot cannot tell who wrote translations that existed before it.
- The record covers the strings glot manages. Content translated elsewhere (database, CMS, emails sent by another tool) needs its own record.
