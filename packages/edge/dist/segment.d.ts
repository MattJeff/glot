export type ValueKind = "number" | "url" | "code" | "public" | "email" | "phone" | "card" | "iban" | "uuid" | "token" | "long_number";
export interface Value {
    kind: ValueKind;
    value: string;
}
export type Placeholder = {
    pair: true;
    name: string;
    open: [number, number];
    close: [number, number];
} | {
    pair: false;
    name: string;
    range: [number, number];
};
export interface Segment {
    hash: string;
    text: string;
    role: string;
    key?: string;
    discoverable: boolean;
    values: Value[];
    tags: Placeholder[];
    span: [number, number];
    attr?: string;
}
export interface SegmentOpts {
    sourceLocale?: string;
    skip?: string[];
    dnt?: string[];
    piiAllow?: string[];
}
export declare const isPii: (k: ValueKind) => boolean;
export declare const allWs: (s: string) => boolean;
export declare function collapseWs(s: string): string;
export declare function decodeEntities(s: string): string;
export declare const escapeText: (s: string) => string;
export declare const escapeAttr: (s: string) => string;
export declare const hasLetter: (s: string) => boolean;
export declare function looksLikeAddress(text: string): boolean;
/** Replace values in `text` with `{n}` (n continues from values.length). */
export declare function mask(text: string, values: Value[], allow: string[]): string;
interface Selector {
    tag?: string;
    id?: string;
    classes: string[];
    attrs: [string, string | undefined][];
}
export declare function parseSelector(s0: string): Selector | undefined;
export declare const CODE_TAGS: string[];
export interface Attr {
    name: string;
    value: string;
    range: [number, number];
}
export type Tok = {
    t: "start";
    name: string;
    attrs: Attr[];
    selfClosing: boolean;
    range: [number, number];
} | {
    t: "end";
    name: string;
    range: [number, number];
} | {
    t: "text";
    range: [number, number];
} | {
    t: "comment";
    range: [number, number];
};
export declare function tokenize(s: string): Tok[];
export declare const VOID: string[];
export declare const INLINE: string[];
export declare const MEDIA: string[];
export declare const ROLE_INLINE: string[];
export declare const META_NAMES: string[];
/** Text without `<k>`, `</k>`, `<k/>`, `{k}`. */
export type Part = {
    p: "text";
    s: string;
} | {
    p: "open" | "close" | "void" | "val";
    n: number;
};
export declare function parseTemplate(t: string, ntags: number, nvalues: number): Part[];
export declare const stripPlaceholders: (t: string) => string;
/** All segments of `html` in document order; `hash` is "" (see segmentHashed). */
export declare function segment(html: string, opts?: SegmentOpts): Segment[];
/** `sha256(source_locale + "\0" + text)`, first 16 bytes hex: the TM key (glot_core::edge::tm::text_hash). */
export declare const segmentKey: (sourceLocale: string, text: string) => string;
export declare function segmentHashed(html: string, opts?: SegmentOpts): Segment[];
/** SHA-256 of `s` (UTF-8), first 16 bytes in hex, synchronously (the browser patcher can't await in a MutationObserver). */
export declare function hash16(s: string): string;
/** Page template (glot_core::edge::tm::template): locale prefix dropped, ids → `:id`, slug leaf → `*`. */
export declare function template(path: string, locale: string): string;
/** Translation template → parts, or undefined unless every placeholder of `seg` appears exactly once, nested right. */
export declare function check(seg: {
    tags: readonly {
        pair: boolean;
    }[];
    values: readonly unknown[];
    attr?: unknown;
}, tr: string): Part[] | undefined;
/** Translate `html`: segments whose lookup returns a valid translation (same placeholders) are replaced, the rest
 *  is left byte for byte. `lookup` gets hashed segments. */
export declare function translateHtml(html: string, opts: SegmentOpts, lookup: (s: Segment) => string | undefined): string;
export {};
