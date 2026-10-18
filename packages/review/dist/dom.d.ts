export declare function decodeKeys(text: string): {
    keys: string[];
    text: string;
};
export declare const norm: (s: string) => string;
/** Visible text of an element, markers stripped. */
export declare const textOf: (el: Element) => string;
/** Element holding the text under the pointer: climbs out of inline wrappers (<b> in a sentence). */
export declare function textTarget(el: Element | null): HTMLElement | null;
/** Key rendered by `el` and how we know: [data-glot-key] (closest), else a dev marker. */
export declare function keyOf(el: Element): {
    key?: string;
    key_source?: "attribute" | "marker";
};
/** Robust selector (contract §1): #id stable > [data-testid] > [data-glot-key] > tag:nth-of-type path from the closest stable ancestor. */
export declare function selectorOf(el: Element): string;
/** Tag path (no text), like @glot/feedback: `main>section#pricing>h2`. */
export declare function domPath(el: Element): string;
/** Every visible text element of the page, document order (keyboard selection mode). */
export declare function candidates(host: Element): HTMLElement[];
type Anchor = {
    key?: string | null;
    selector?: string | null;
    text?: string | null;
};
/** Re-anchor (contract §1): key whose text ≈ text > selector > text search. null = orphan. */
export declare function findAnchor(a: Anchor, all: HTMLElement[]): HTMLElement | null;
/** ICU arguments and tags of a message, top level: `{name}`, `{count, plural, …}`, `<b>`. Order-free. */
export declare function placeholders(s: string): string[];
/** Segments of `s` with placeholders flagged (for highlighting). */
export declare function segments(s: string): [string, boolean][];
/**
 * Cropped picture of `el` and its surroundings: the closest block ancestor, serialized with computed styles
 * into an SVG <foreignObject>, painted on a canvas, target outlined. Best effort: null when tainted / too big.
 */
export declare function capture(el: HTMLElement): Promise<Blob | null>;
export {};
