import { type Value } from "./segment.js";
import { type Tm } from "./edge.js";
export { hash16, template } from "./segment.js";
/** `skip` = the project's excluded selectors (sent along with the TM by the JS runtimes). */
export type PatcherTm = Tm & {
    skip?: string[];
};
type Tok = {
    p: "open" | "close" | "void";
    n: number;
    e: Element;
};
export interface DomSegment {
    text: string;
    role: string;
    hash: string;
    discoverable: boolean;
    values: Value[];
    tags: {
        pair: boolean;
    }[];
    /** Text segment: placeholders in order + the text nodes between them. */
    toks: Tok[];
    gaps: Text[][];
    /** Attribute segment: element + attribute name. */
    el?: Element;
    attr?: string;
}
/** Segments of `root` (and its descendants), as segment.ts would cut the same markup. */
export declare function domSegments(root: Element, source: string, skip?: string): DomSegment[];
export interface PatcherOptions {
    source: string;
    tmUrl?: string;
    inline?: string | null;
    pageVersion?: number;
    storage?: Storage | null;
    /** Discovery endpoint (`/__glot/missing`); none → no discovery. */
    missing?: string;
    /** TM re-poll delays after a report, ms. */
    poll?: number[];
}
export declare function startPatcher(o: PatcherOptions): {
    mo: MutationObserver;
    readonly tm: PatcherTm | undefined;
    flush: () => void;
    send: () => void;
};
