// AI-translation provenance: read `messages/{locale}.provenance.json` (written by `glot provenance catalog`)
// and label machine translations, e.g. "Translated by AI · reviewed on 3 Sep 2026". No framework.

export interface ProvenanceEntry {
  origin: "machine" | "human" | "imported";
  model?: string;
  /** RFC 3339 UTC. */
  translated_at?: string;
  reviewed: boolean;
  reviewed_at?: string;
  reviewed_by?: string;
  /** Judge score 0-100. */
  score?: number;
}

/** `{ "<dotted key>": ProvenanceEntry }` */
export type ProvenanceCatalog = Record<string, ProvenanceEntry>;

export interface ProvenanceLabels {
  /** Default "Translated by AI". */
  ai?: string;
  /** `{date}` is replaced. Default "reviewed on {date}". */
  reviewed?: string;
}

/** Entry for a key, or undefined (unknown key, or no catalog). */
export function provenanceOf(catalog: ProvenanceCatalog | undefined, key: string): ProvenanceEntry | undefined {
  return catalog && Object.prototype.hasOwnProperty.call(catalog, key) ? catalog[key] : undefined;
}

/** Produced by a model (reviewed or not). Human-written and imported strings are not. */
export function isMachineTranslated(e: ProvenanceEntry | undefined): boolean {
  return e?.origin === "machine";
}

/** True when any of `keys` (or the whole catalog) holds a machine translation: show a page-level notice. */
export function hasMachineTranslations(catalog: ProvenanceCatalog | undefined, keys?: string[]): boolean {
  if (!catalog) return false;
  return (keys ?? Object.keys(catalog)).some((k) => isMachineTranslated(provenanceOf(catalog, k)));
}

/**
 * "Translated by AI" / "Translated by AI · reviewed on <date>" for a machine translation, `null` otherwise.
 * Pass translated `labels` (e.g. from your own catalog: `{ ai: "Traduit par IA", reviewed: "relu le {date}" }`).
 */
export function provenanceLabel(
  e: ProvenanceEntry | undefined,
  o: { locale?: string; labels?: ProvenanceLabels } = {},
): string | null {
  if (!isMachineTranslated(e)) return null;
  const ai = o.labels?.ai ?? "Translated by AI";
  const when = e!.reviewed ? e!.reviewed_at : undefined;
  if (!when) return ai;
  const d = new Date(when);
  const date = Number.isNaN(d.getTime())
    ? when
    : new Intl.DateTimeFormat(o.locale, { dateStyle: "medium", timeZone: "UTC" }).format(d);
  return `${ai} · ${(o.labels?.reviewed ?? "reviewed on {date}").replace("{date}", date)}`;
}

/** Small inline `<small>` badge for a machine translation (null otherwise, or without a DOM). */
export function provenanceBadge(
  e: ProvenanceEntry | undefined,
  o: { locale?: string; labels?: ProvenanceLabels } = {},
): HTMLElement | null {
  const text = provenanceLabel(e, o);
  if (!text || typeof document === "undefined") return null;
  const el = document.createElement("small");
  el.className = "glot-provenance";
  el.dataset.origin = e!.origin;
  if (o.locale) el.lang = o.locale;
  el.textContent = text;
  return el;
}
