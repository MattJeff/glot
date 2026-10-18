type Json = {
    [k: string]: unknown;
};
export interface OnDemandOptions {
    /** glot server base URL, e.g. `https://glot.example.com`. */
    server: string;
    /** Project slug (its CDN must be public, and on-demand enabled for the locale). */
    project: string;
    /** Source catalog: missing keys in a partial catalog fall back to it (deep merge). */
    source?: Json;
    /** How long a catalog is served from memory before revalidating (ETag) in the background. Default 60 s. */
    ttl?: number;
    fetch?: typeof fetch;
}
/** Returns `load(locale)`: the translated catalog, or `null` while glot translates it (or it's not allowed). */
export declare function createOnDemandLoader({ server, project, source, ttl, fetch: f }: OnDemandOptions): (locale: string) => Promise<Json | null>;
/** `Accept-Language` → language tags by preference (`*` and q=0 dropped). */
export declare function acceptedLanguages(header: string | null | undefined): string[];
export {};
