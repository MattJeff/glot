import type { ReviewOptions } from "./types";
export type { ReviewOptions, Labels } from "./types";
/** Review token for this tab, or "" (normal visitor). Strips `glot_review` from the address bar. */
export declare function reviewToken(explicit?: string): string;
/** Starts review mode if the visitor is a reviewer. Resolves to true when the widget was loaded. */
export declare function init(opts: ReviewOptions): Promise<boolean>;
export declare const glotReview: {
    init: typeof init;
};
