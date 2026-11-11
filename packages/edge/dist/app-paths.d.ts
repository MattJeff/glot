export type AppAction = "redirect" | "proxy";
export interface AppRules {
    rules: string[];
    action?: AppAction;
    /** `query:lang` | `prefix` | `none` (from the project's preview url_pattern). */
    locale?: string;
}
export type AppDecision = {
    kind: "proxy";
} | {
    kind: "redirect";
    status: number;
    location: string;
    cookie: string;
};
export declare const HOP_COOKIE = "glot_app_hop";
/** One rule line → [action, lowercase pattern]; undefined for blank / `#` / invalid lines. */
export declare function parseLine(line: string, def: AppAction): [AppAction, string] | undefined;
/** Normalized path: starts with `/`, ASCII only, no `//`, `.` or `..` segments. */
export declare function normalize(path: string): string;
/** The action of the first rule matching `path` (no query). glot's own files are never app paths. */
export declare function classify(rules: readonly string[], def: AppAction, path: string): AppAction | undefined;
export interface AppInput {
    rules: readonly string[];
    action: AppAction;
    locale_style: string;
    locale: string;
    /** Where the source pages live (scheme://host[:port] is kept, nothing else). */
    origin: string;
    /** This request's host and public path (with the locale prefix, if any). */
    host: string;
    req_path: string;
    /** Origin path (locale prefix removed) and raw query (no `?`). */
    path: string;
    query: string;
    method: string;
    cookie: string;
}
export declare function decide(i: AppInput): AppDecision | undefined;
