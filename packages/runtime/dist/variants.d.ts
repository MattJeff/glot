export declare function readVariants(): Record<string, string>;
/** True once @glot/insights has this session's assignment (legacy client-side draw). */
export declare function hasVariantState(): boolean;
/** Keeps the edge's segmented-experiment assignments (contract §7) where `@glot/insights` `track()` reads them. */
export declare function storeAssignments(a: {
    id: string;
    cell: string;
    arm: number;
}[]): void;
/** `messages` with each dotted key replaced by its variant; only keys that already exist as strings are touched. */
export declare function withVariants<M extends Record<string, unknown>>(messages: M, variants: Record<string, string>): M;
