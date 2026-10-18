export declare function readVariants(): Record<string, string>;
/** `messages` with each dotted key replaced by its variant; only keys that already exist as strings are touched. */
export declare function withVariants<M extends Record<string, unknown>>(messages: M, variants: Record<string, string>): M;
