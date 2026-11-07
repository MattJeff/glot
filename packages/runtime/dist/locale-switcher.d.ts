export type Loc = {
    code: string;
    name: string;
    dir: "ltr" | "rtl";
    coverage: number;
};
export type Manifest = {
    source: string;
    locales: Loc[];
    updated_at: string | null;
};
type Json = {
    [k: string]: unknown;
};
/** Picks the manifest locale matching `tag` exactly, else by base language (`fr-CA` → `fr`). */
export declare function match(tag: string | null | undefined, codes: string[]): string | undefined;
/** `auto` resolves to `query:lang` when the URL has `?lang=`, `prefix` when the path starts with a locale, else `event`. */
export declare function resolveStrategy(strategy: string, url: URL, codes: string[]): string;
/** The locale the page is in, per strategy (`event`: the saved choice); falls back to <html lang>, the saved choice, the source. */
export declare function currentLocale(strategy: string, url: URL, cookie: string, htmlLang: string, saved: string | null, m: Manifest): string;
/** Where to go for `code` under a navigating strategy (`query:*`, `prefix`, `prefix:always`); null otherwise. */
export declare function targetUrl(strategy: string, url: URL, code: string, m: Manifest): string | null;
/** Fetches a published catalog (nested JSON) and announces it with a `glot:catalog` event on `document`. */
export declare function loadCatalog(locale: string, opts?: {
    project?: string;
    cdn?: string;
}): Promise<Json>;
export {};
