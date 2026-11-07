export type Lookup = (normalizedSource: string) => string | undefined;
/** Translate the framework payloads of `html` with the page's segment lookup. `replaced` = strings changed. */
export declare function translatePayloads(html: string, lookup: Lookup): {
    html: string;
    replaced: number;
};
/** `<script async>` of the client patcher (glot_core::edge::hydration::patcher_tag); `missing` = discovery endpoint. */
export declare const patcherTag: (scriptUrl: string, tmUrl: string, sourceLocale: string, missing?: string) => string;
/** Insert `tag` after `<meta name="glot-tm" …>`, else right after `<head>` (glot_core::edge::hydration::inject_head). */
export declare function injectHead(html: string, tag: string): string;
