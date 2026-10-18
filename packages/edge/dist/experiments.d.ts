export type Rule = {
    all: {
        attr: string;
        eq?: string;
        in?: string[];
        not_in?: string[];
    }[];
};
export type XpConfig = {
    v: 1;
    generated_at: string;
    /** Set by the loader on each 200/304 (contract v1.3: `max_stale` counts from there, the ETag ignores `generated_at`). */
    fetched_at?: number;
    max_stale: number;
    persistence: "none" | "session";
    holdout: {
        id: string;
        pct: number;
    } | null;
    segments: {
        id: string;
        rule: Rule;
    }[];
    experiments: {
        id: string;
        status: string;
        traffic_pct: number;
        locales: string[];
        /** Keys under test (contract v1.6): forwarded as `x-glot-xp.k` so the runtime can measure "seen". */
        keys?: string[];
        cells: {
            id: string;
            weights: number[];
            count: boolean;
        }[];
        arms: {
            arm: number;
            values?: Record<string, Record<string, string>>;
        }[];
    }[];
};
export type Assignment = {
    id: string;
    cell: string;
    arm: number;
};
export type Decision = {
    /** Every assignment of this document, holdout included (`x-glot-xp.a`). */
    assignments: Assignment[];
    /** key → text for the served locale (arm 0 never appears). */
    overlay: Record<string, string>;
    /** What to count: empty for bots, prefetches, opt-out, `?glot_xp=` previews, personalized cells. */
    exposures: Assignment[];
    /** `x-glot-xp` request header for the app, or null (= official messages). */
    header: string | null;
    /** `Set-Cookie` values to add to the response. */
    cookies: string[];
};
export type DecideOptions = {
    /** Locale being served (from `route()`). */
    locale: string;
    /** Consent signal from your CMP: only then is a persistent cookie used (`visitor` = new/returning). */
    consent?: boolean;
    /** App attributes (`plan`, `logged_in`, `custom.*`): matched here, never sent to glot. */
    attrs?: Record<string, string>;
    /** Stable unit to hash (an id you already have): same unit ⇒ same arms. Default: random per document. */
    unit?: string;
    salt?: string;
    now?: number;
    rand?: () => number;
};
/** A real document navigation (counted once): not a prefetch, not a Next RSC / client-navigation fetch.
 * Those get the same decision (same header, no flicker), they're just not exposures. */
export declare function isDocument(req: Request, url?: URL): boolean;
export declare const isBot: (ua: string | null) => boolean;
/** Visitor context (contract §3): country, region, locale, device, source, visitor + app attributes. */
export declare function context(req: Request, o: Pick<DecideOptions, "locale" | "consent" | "attrs">, url?: URL): Record<string, string>;
/** Contract §3: conjunction; absent attribute ⇒ `eq`/`in` false, `not_in` true. */
export declare const matches: (rule: Rule, ctx: Record<string, string>) => boolean;
/** FNV-1a → [0, 1). Deterministic across runtimes. */
export declare function hash(s: string): number;
/** Index into `weights` for u ∈ [0, 1); -1 if every weight is 0. */
export declare function pick(weights: number[], u: number): number;
export declare function decide(req: Request, config: XpConfig | null, o: DecideOptions): Decision;
/** base64url, no padding, of UTF-8 (contract §7). */
export declare function encode(s: string): string;
export type LoaderOptions = {
    /** glot server base URL. */
    server: string;
    /** Project slug. */
    project: string;
    /** Served from memory this long before a background revalidation (ETag). Default 60 s. */
    ttl?: number;
    /** Cold start: max wait for the first config, then official text. Default 150 ms. */
    timeout?: number;
    fetch?: typeof fetch;
};
type WaitUntil = (p: Promise<unknown>) => void;
/** `load(waitUntil?)` → the config, or null (none yet / glot unreachable / not public). Never throws. */
export declare function createExperimentsLoader({ server, project, ttl, timeout, fetch: f }: LoaderOptions): (waitUntil?: WaitUntil) => Promise<XpConfig | null>;
export type ExperimentsOptions = LoaderOptions & {
    /** Project public key (`pk_…`, dashboard → Insights): exposures go to `POST /api/insights/{key}`. */
    key: string;
    /** Your CMP's consent signal for this request. Default: none (no persistent cookie). */
    consent?: (req: Request) => boolean;
    /** How long exposures wait to be batched (ms, default 200; inside `waitUntil`, after the response). */
    batchMs?: number;
    /** App attributes for segments (`plan`, `logged_in`, `custom.*`). */
    attrs?: (req: Request) => Record<string, string> | undefined;
};
/** Load + decide + report exposures in the background. Shared by the Next proxy, the Worker and any edge runtime. */
export declare function glotExperiments(o: ExperimentsOptions): (req: Request, locale: string, waitUntil?: WaitUntil) => Promise<Decision>;
/** Request headers for the app: incoming `x-glot-xp` always dropped (client-forged), ours set when assigned. */
export declare function forwardHeaders(headers: Headers, d: Decision): Headers;
export {};
