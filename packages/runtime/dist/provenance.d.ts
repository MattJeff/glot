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
export declare function provenanceOf(catalog: ProvenanceCatalog | undefined, key: string): ProvenanceEntry | undefined;
/** Produced by a model (reviewed or not). Human-written and imported strings are not. */
export declare function isMachineTranslated(e: ProvenanceEntry | undefined): boolean;
/** True when any of `keys` (or the whole catalog) holds a machine translation: show a page-level notice. */
export declare function hasMachineTranslations(catalog: ProvenanceCatalog | undefined, keys?: string[]): boolean;
/**
 * "Translated by AI" / "Translated by AI · reviewed on <date>" for a machine translation, `null` otherwise.
 * Pass translated `labels` (e.g. from your own catalog: `{ ai: "Traduit par IA", reviewed: "relu le {date}" }`).
 */
export declare function provenanceLabel(e: ProvenanceEntry | undefined, o?: {
    locale?: string;
    labels?: ProvenanceLabels;
}): string | null;
/** Small inline `<small>` badge for a machine translation (null otherwise, or without a DOM). */
export declare function provenanceBadge(e: ProvenanceEntry | undefined, o?: {
    locale?: string;
    labels?: ProvenanceLabels;
}): HTMLElement | null;
