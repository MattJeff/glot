export type Syntax = "icu" | "i18next" | "mf2";
export interface Term {
    term: string;
    do_not_translate?: boolean;
    case_sensitive?: boolean;
    translations?: Record<string, string>;
    description?: string;
    forbidden?: Record<string, string[]>;
}
/** `glossary.toml` parsed (`{ term: [...] }`) or the API's `Term[]`. */
export type Glossary = Term[] | {
    term?: Term[];
};
export declare const terms: (g?: Glossary) => Term[];
type Node = string | {
    raw: string;
} | Choice;
interface Choice {
    head: string;
    kind: "plural" | "selectordinal" | "select";
    options: [string, Node[]][];
    pound: boolean;
}
/** ICU MessageFormat (FormatJS flavour: tags, DOUBLE_OPTIONAL apostrophes). Throws on syntax errors. */
export declare function parseIcu(s: string): Node[];
export declare function parse(s: string, syntax: Syntax): Node[] | null;
export declare const lookup: <T>(m: Record<string, T> | undefined, locale: string) => T | undefined;
type Slot = {
    raw: string;
} | Choice;
export interface Unit {
    /** Full masked text (tokens included). */
    text: string;
    lead: string;
    /** What goes to the translator. Empty => nothing to translate (no letters). */
    core: string;
    trail: string;
    slots: Slot[];
    syntax: Syntax;
    pound: boolean;
}
export interface Masked {
    /** Every unit of the message, branches included (dedupe on `core` before translating). */
    units: Unit[];
    /** Rebuild the message from `translate(unit) → string`; null if any unit lost/duplicated a token. */
    restore(get: (u: Unit) => string): string | null;
}
/** Mask one message for `locale`. Returns null when the message can't be parsed (it stays in source). */
export declare function mask(message: string, opts: {
    syntax?: Syntax;
    glossary?: Glossary;
    locale: string;
}): Masked | null;
/** i18next plural keys: `x_other` → add the target locale's missing `x_few`/`x_many`… (copy of `x_other`). */
export declare function i18nextPluralKeys(flat: Record<string, string>, locale: string): Record<string, string>;
export {};
