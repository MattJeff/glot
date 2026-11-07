import { type Translator } from "./edge.js";
export declare const translate: Translator;
export interface SeoCtx {
    source: string;
    /** Locale this page is served in. */
    locale: string;
    /** `[locale, base URL]` of every READY locale, the source first; a locale missing here gets no hreflang. */
    bases: [string, string][];
    /** Path + query of the page in source terms. */
    path: string;
    /** Translates one JSON-LD free text; undefined keeps the source. */
    tr: (text: string) => string | undefined;
}
/** Subdirectory mode bases: the source at the origin, `{origin}/{locale}` for the others. */
export declare const subdirBases: (origin: string, source: string, locales: readonly string[]) => [string, string][];
export declare const dir: (l: string) => "rtl" | "ltr";
/** Source-terms path of a link to this site (any locale's base); undefined for external links, fragments, assets. */
export declare function sitePath(s: SeoCtx, href0: string): string | undefined;
/** Internal link for the served locale: root-relative stays root-relative (subdirectory: gains the prefix). */
export declare function localizeHref(s: SeoCtx, href: string): string | undefined;
/**
 * International SEO of one page (glot_core::edge::seo::rewrite): `<html lang dir>`, one canonical (the origin's,
 * localized), reciprocal hreflang for every ready locale + x-default, og:locale / og:url, JSON-LD texts through the
 * TM and its URLs localized, internal links localized. Origin-managed alternates (`?lang=en`) stay, made absolute.
 */
export declare function seo(html: string, s: SeoCtx): string;
