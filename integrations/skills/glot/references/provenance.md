# AI-translation provenance

`glot.lock` records, per locale and key: `origin` (`machine` with its `model`, `human`, `imported`), `translated_at`, `reviewed`, `reviewed_at`, `reviewed_by` (from `GLOT_REVIEWER`) and the judge `score`.

- Audit export: `glot provenance export --format csv > provenance.csv` (also `json`, `html`).
- Runtime: `glot provenance catalog` writes `{locale}.provenance.json` next to each catalog; in the app,
  `import { provenanceLabel } from "@glot-i18n/runtime/provenance"` renders "Translated by AI · reviewed on …".

This is information to help the team meet transparency duties such as Article 50 of the EU AI Act (applicable from 2 August 2026). Whether and how a disclosure is required depends on the use case: do not state legal conclusions to the user; point them to their counsel.
