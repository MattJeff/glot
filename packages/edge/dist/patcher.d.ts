import { type Value, type Part } from "./segment.js";
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
/** Write `parts` (a checked translation) into the live nodes of `seg`; false if its placeholder order differs. */
export declare function write(seg: DomSegment, parts: Part[], mine: WeakMap<object, string>): boolean;
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
    /** Project excluded selectors when the TM doesn't carry them (app mode reads glot's TM directly). */
    skip?: string[];
    /**
     * App mode (pages behind sign-in, `app.global.ts`): there is no server pass, so every text is the app's and a
     * session cookie doesn't stop discovery — but only text inside `allow` is a candidate, and only `[data-glot-ui]`
     * text is reported as text; any other candidate goes as its hash alone (glot keeps it only if it already knows
     * that text from a public page: a user's name never matches).
     */
    app?: {
        allow: string;
    };
    /**
     * Website tag (`website.global.ts`): no server pass, every text of the page is a candidate (not only what the
     * app rendered after boot); still never with a session cookie, and in a `<form>` only labels/legends/buttons.
     */
    all?: boolean;
    /** Extra fields of the discovery report (`key`, `auth`, `mode`). */
    report?: Record<string, unknown>;
    /** Website tag: paths where discovery is allowed (project zones, private-looking paths refused). */
    zone?: (path: string) => unknown;
    /** Called each time a TM has been applied to the page (the tag un-hides it). */
    ready?: () => void;
}
export declare function startPatcher(o: PatcherOptions): {
    mo: MutationObserver;
    readonly tm: PatcherTm | undefined;
    flush: () => void;
    send: () => void;
    scan: () => void;
};
