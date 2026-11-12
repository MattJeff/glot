/** `site_code`: the code the site itself uses for the locale, when it differs from glot's (`zh` for `zh-CN`). */
export type Loc = {
    code: string;
    name: string;
    dir: "ltr" | "rtl";
    coverage: number;
    site_code?: string;
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
/** The flag for a locale ("" = none, show the code badge). `over` keys are lowercase codes; values an emoji or a region (`us`). */
export declare function flagFor(code: string, over?: Record<string, string>): string;
/** Fetches a published catalog (nested JSON) and announces it with a `glot:catalog` event on `document`. */
export declare function loadCatalog(locale: string, opts?: {
    project?: string;
    cdn?: string;
}): Promise<Json>;
/** Did the page take the language? `<html lang>` names it; or the site keeps `lang` as it was and the text changed. */
export declare function applied(lang: string, loc: Pick<Loc, "code" | "site_code">, before: {
    l: string;
    p: number;
}, now: number, source: string): boolean;
export {};
