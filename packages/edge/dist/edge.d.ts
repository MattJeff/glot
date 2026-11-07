/** TM snapshot, wire format of `GET /edge/v1/{project}/{locale}/tm` (glot_core::edge::tm): segment hash →
 * one translation valid in any context, or per context (`<role>|<page template>`, `*` = catalog default). */
export interface Tm {
    v: number;
    locale: string;
    t: Readonly<Record<string, string | Readonly<Record<string, string>>>>;
}
/** Exact context, then the catalog default; an ambiguous text in a new context has no translation. */
export declare function lookup(tm: Tm, hash: string, context: string): string | undefined;
export interface Status {
    enabled: boolean;
    /** Hosts where Edge is switched off (per-domain kill switch). */
    off: string[];
    /** Current TM version per locale. */
    versions: Record<string, number>;
    /** Project CSS selectors never extracted (user content, widgets). */
    selectors?: {
        include: string[];
        exclude: string[];
    };
    /** Locales ready to be listed (hreflang), from `/cdn/{project}/locales.json`; undefined = unknown (all listed). */
    ready?: string[];
}
/** A segment glot has no translation for (from the segmenter); `discoverable: false` ones are never sent. */
export interface Missing {
    hash: string;
    text: string;
    /** Role of the carrying element (`button`, `a`, `h1`, `title`…): the server derives the context from it. */
    role?: string;
    discoverable?: boolean;
}
export interface TranslateResult {
    html: string;
    /** Rendered segments / translated ones (coverage of this page). */
    total: number;
    translated: number;
    missing: Missing[];
    /** Every rendered segment (per-URL coverage), optional. */
    seen?: {
        hash: string;
        role: string;
    }[];
}
export interface TranslateCtx {
    /** Public (localized) URL of the page. */
    url: URL;
    /** Origin path of the page (source terms, no locale prefix, no query). */
    path: string;
    locale: string;
    source: string;
    /** Locales listed as hreflang alternates: the ready ones + this page's. */
    locales: readonly string[];
    /** Selectors never extracted. */
    skip: string[];
    /** Client discovery on (project key set, anonymous visitor): the patcher tag gets `data-missing`. */
    discover?: boolean;
}
/** Synchronous: one TM snapshot in, one page out (translate.ts). */
export type Translator = (html: string, tm: Tm, ctx: TranslateCtx) => TranslateResult;
/** Persistent last-known state (Worker: Cache API; Node: a directory). */
export interface Store {
    get(key: string): Promise<string | null>;
    put(key: string, value: string): Promise<void>;
}
/** Shared page cache (Worker: `caches.default`). Keys already contain locale and version. */
export interface PageCache {
    match(key: string): Promise<Response | undefined>;
    put(key: string, res: Response): Promise<void>;
}
export interface EdgeOptions {
    /** glot server, e.g. `https://api.glotlang.com`. */
    server: string;
    project: string;
    /** Project public key (`pk_…`): sent with discovery reports. No key → no discovery. */
    key?: string;
    source: string;
    /** Locales served besides the source (hreflang alternates). */
    locales?: readonly string[];
    translate: Translator;
    /** Budget for glot on a cold start, ms (default 300). */
    timeout?: number;
    /** Status revalidation period, ms (default 20 000: kill switch < 1 min including the server's cache). */
    statusTtl?: number;
    /** Larger HTML pages are streamed through untranslated (default 3 MB). */
    maxHtml?: number;
    store?: Store;
    pageCache?: PageCache;
    /** Seconds a translated page is kept in `pageCache` when the origin doesn't say (default 0 = obey the origin only). */
    pageTtl?: number;
    fetch?: typeof fetch;
    now?: () => number;
}
type Wait = (p: Promise<unknown>) => void;
export declare const SESSION_COOKIE: RegExp;
export declare function createEdge(o: EdgeOptions): ((req: Request, locale: string, originUrl: URL, origin: (u: URL) => Promise<Response>, wait?: Wait) => Promise<Response>) & {
    own: (req: Request, wait?: Wait) => Promise<Response | undefined>;
};
/** Seconds the origin allows a shared cache to keep this page (0 = not shareable). */
export declare function cacheTtl(h: Headers, fallback: number): number;
export {};
