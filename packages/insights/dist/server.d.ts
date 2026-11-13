export type ServerOptions = {
    key: string;
    endpoint: string;
    served: string;
};
/** Languages from Accept-Language, ordered by q. */
export declare function acceptLanguages(header: string | null): string[];
/** Posts one visit. Returns the locale to serve when an experiment or a launch holdout assigns one (the new locale,
 * or the fallback for held-out visitors). `explicit`: the visitor picked the locale (cookie / URL set by your
 * switcher): never assigned. */
export declare function reportVisit(req: Request, o: ServerOptions & {
    event?: string;
    arm?: string;
    explicit?: boolean;
}): Promise<{
    locale: string | null;
    arm: string | null;
} | null>;
