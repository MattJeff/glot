import type { IncomingMessage, ServerResponse } from "node:http";
import { type RouteOptions } from "./route.js";
import { type EdgeOptions, type Store } from "./edge.js";
export type ExpressEdgeOptions = RouteOptions & Omit<EdgeOptions, "source" | "translate"> & {
    /** Where to fetch this app's pages; default `http://127.0.0.1:{the server's port}`. */
    origin?: string;
    translate?: EdgeOptions["translate"];
};
/** Last known TM/status as files in `dir` (atomic rename). */
export declare function fileStore(dir: string): Store;
export declare function glotEdge(o: ExpressEdgeOptions): (req: IncomingMessage, res: ServerResponse, next: (e?: unknown) => void) => Promise<void>;
