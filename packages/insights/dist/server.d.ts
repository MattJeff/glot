export type ServerOptions = {
    key: string;
    endpoint: string;
    served: string;
};
/** Languages from Accept-Language, ordered by q. */
export declare function acceptLanguages(header: string | null): string[];
/** Posts one visit. Returns the experiment locale to serve when this visitor lands in the test arm. */
export declare function reportVisit(req: Request, o: ServerOptions & {
    event?: string;
    arm?: string;
}): Promise<{
    locale: string | null;
    arm: string | null;
} | null>;
