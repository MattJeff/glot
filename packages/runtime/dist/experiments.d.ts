/** One assignment: experiment (or holdout) id, segment cell (`*` = rest of traffic), arm (0 = official). */
export type Assignment = {
    id: string;
    cell: string;
    arm: number;
};
/** `k` (optional, contract v1.6): experiment id → its keys, to find the tested blocks in the page ("seen" exposure). */
export type XpHeader = {
    v: 1;
    locale: string;
    a: Assignment[];
    o?: Record<string, string>;
    k?: Record<string, string[]>;
};
/** Published config (`GET /cdn/{slug}/experiments.json`), only what's needed to rebuild a missing overlay. */
export type XpConfig = {
    experiments: {
        id: string;
        locales: string[];
        arms: {
            arm: number;
            values?: Record<string, Record<string, string>>;
        }[];
    }[];
};
export interface WithExperimentsOptions {
    /** Raw `x-glot-xp` request header (`(await headers()).get("x-glot-xp")`). */
    header: string | null | undefined;
    /** Locale actually being served. When it differs from the edge's locale (e.g. on-demand catalog not ready yet),
     *  the overlay is skipped: official text, never a variant in the wrong language. */
    locale?: string;
    /** Experiments config: rebuilds the overlay when the edge sent assignments only (`o` absent). */
    config?: XpConfig | null;
}
/** Decodes `x-glot-xp`; null when absent or unreadable (→ official messages). */
export declare function readXpHeader(header: string | null | undefined): XpHeader | null;
/** Encodes an `x-glot-xp` value (tests, custom proxies). */
export declare function writeXpHeader(x: XpHeader): string;
/** `messages` with the edge's variant overlay applied (only existing string keys; arm 0 = untouched). */
export declare function withExperiments<M extends Record<string, unknown>>(messages: M, o: WithExperimentsOptions): M;
export type Seen = Assignment & {
    seen: true;
    visible_ms: number;
};
/** Visible time rounded down to a coarse bucket (aggregated stats only, nothing personal). */
export declare const bucket: (ms: number) => number;
/**
 * Watches the blocks of each experiment (`[data-glot-key]`, or the Worker's `[data-glot]`, whose key is in `keys[id]`).
 * An experiment is "seen" once one of its blocks stayed ≥ 50 % visible for ≥ 1 s; its cumulative visible time is
 * reported (bucketed) when the page is hidden or left. `onSeen` fires at most once per experiment and document.
 */
export declare function observeSeen(a: Assignment[], keys: Record<string, string[]>, onSeen: (s: Seen) => void): () => void;
