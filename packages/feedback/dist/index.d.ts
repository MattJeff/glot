export type Labels = {
    report: string;
    title: string;
    text: string;
    wrong: string;
    awkward: string;
    unclear: string;
    suggestion: string;
    send: string;
    cancel: string;
    thanks: string;
};
export type Options = {
    /** Project public key (dashboard → Feedback, same as Insights). */
    key: string;
    /** glot server origin. */
    endpoint?: string;
    /** Locale the app rendered (default: <html lang>). */
    locale?: string;
    /** Pass them translated in the user's language. */
    labels?: Partial<Labels>;
};
/** Open the form, prefilled with `text` (default: the current selection). */
export declare function open(text?: string, key?: string): void;
export declare function init(opts: Options): void;
export declare const glotFeedback: {
    init: typeof init;
    open: typeof open;
};
