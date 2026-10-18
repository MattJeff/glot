export interface RouteOptions {
    source: string;
    /** Locales served under `/{locale}/…`: shipped ones + those enabled on demand in glot. Required, so `/api` or `/faq` is never taken for a locale. */
    locales: readonly string[];
}
export type Route = {
    redirect: string;
} | {
    locale: string;
    path: string;
};
export declare function route(url: URL, headers: Headers, { source, locales }: RouteOptions): Route;
