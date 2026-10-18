export type LayoutOptions = {
    /** Project public key (dashboard → Insights). */
    key: string;
    endpoint?: string;
    /** Locale the app rendered (default: <html lang>). */
    locale?: string;
    /** Share of page views measured, 0..1 (default 0.01). */
    sample?: number;
    /** Send after this many ms if the page is still open (default 10 s). */
    delay?: number;
};
/** Overflow of one element: [overflows, box px, needed px]. Ellipsis, clip, line-clamp and text spilling
 * out of its box (overlapping neighbours) all show up as scroll size > client size. */
export declare function measure(el: HTMLElement): [boolean, number, number];
/** Starts measuring this page view (if sampled in); returns a stop function. */
export declare function watchLayout(o: LayoutOptions): () => void;
