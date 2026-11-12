import { type LiveOptions } from "./index.js";
export type ObserveOptions = Omit<LiveOptions, "signal"> & {
    /** Quiet time after which a node's text is considered complete. Default 700 ms. */
    settle?: number;
};
export declare function observeLive(root: Element, opts: ObserveOptions): () => void;
