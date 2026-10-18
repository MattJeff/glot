import { type ReviewOptions } from "./types";
/** Route as the server computes it (contract §1): path, no trailing slash, leading locale segment dropped. */
export declare function routeOf(path: string, locale: string): string;
export declare function start(opts: ReviewOptions & {
    token: string;
}): void;
