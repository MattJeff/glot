export type Options = {
    /** Project public key (dashboard → Insights). */
    key: string;
    /** glot server origin. */
    endpoint?: string;
    /** Locale the app rendered (default: <html lang>). */
    locale?: string;
    /** Called when this session is in the test arm of a language experiment: render that locale. */
    onLocale?: (locale: string) => void;
};
/** Copy variant served to this session for a key: experiment id, arm, text. */
export type Variant = {
    id: string;
    arm: number;
    value: string;
};
/** Counts one visit per browser session and applies a running language experiment. */
export declare function init(o: Options): Promise<void>;
/** Conversion event: "signup", "purchase" or any name (1-40 chars of a-z0-9_-.:). Credits this session's
 * language-experiment arm and copy-variant arms. */
export declare function track(event: string): void;
/** Copy variant text served to this session for `key` (a validated variant of a running experiment), if any. */
export declare function variant(key: string): string | undefined;
export declare const glot: {
    init: typeof init;
    track: typeof track;
    variant: typeof variant;
};
