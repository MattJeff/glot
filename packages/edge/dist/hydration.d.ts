export type Lookup = (normalizedSource: string) => string | undefined;
/** A string value of a data literal: `[a, b)` = the literal with its quotes, `s` = decoded. */
export interface Lit {
    a: number;
    b: number;
    s: string;
    key?: string;
    top?: number;
}
/**
 * String values of a JSON text or a JS *data literal* (objects, arrays, calls, arrows, identifiers, numbers) in
 * `src[from..to]`. `one`: `src[from]` is a bracket, stop when it closes. Object keys, computed member names
 * (`x["k"]`) and strings outside any bracket are not returned; each value carries the key it belongs to
 * (inherited through arrays: `title: [0, "…"]`) and, directly in a top-level array, its index. undefined =
 * something a generated data literal never contains (template literal, slash = regex/comment, `?`, unbalanced
 * brackets): the caller leaves that script as is.
 */
export declare function scan(src: string, from?: number, to?: number, one?: boolean): Lit[] | undefined;
/** Translate the framework payloads of `html` with the page's segment lookup. `replaced` = strings changed. */
export declare function translatePayloads(html: string, lookup: Lookup): {
    html: string;
    replaced: number;
};
/** Largest TM inlined in the page (bytes of JSON); a larger one is fetched by the patcher (`data-tm`). */
export declare const TM_INLINE_MAX = 32768;
/** `<script type="application/json" id="glot-tm">` carrying the page's TM when it is at most TM_INLINE_MAX bytes:
 * the patcher translates text the app renders right after load (after-mount, client-only) without waiting for a
 * request, so a cold visit paints no source text. "" when larger (hydration.rs `tm_inline`). */
export declare const tmInline: (json: string) => string;
/** `<script async>` of the client patcher (glot_core::edge::hydration::patcher_tag); `missing` = discovery endpoint. */
export declare const patcherTag: (scriptUrl: string, tmUrl: string, sourceLocale: string, missing?: string) => string;
/** Insert `tag` after `<meta name="glot-tm" …>`, else right after `<head>` (glot_core::edge::hydration::inject_head). */
export declare function injectHead(html: string, tag: string): string;
