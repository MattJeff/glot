import { type NextFetchEvent, type NextRequest } from "next/server.js";
import { type RouteOptions } from "./route.js";
import { type ExperimentsOptions } from "./experiments.js";
import { type EdgeOptions } from "./edge.js";
export type EdgeProxyOptions = Omit<EdgeOptions, "source" | "translate"> & {
    origin?: string;
    translate?: EdgeOptions["translate"];
};
export declare function glotProxy(o: RouteOptions & {
    experiments?: ExperimentsOptions;
    edge?: EdgeProxyOptions;
}): (req: NextRequest, event?: NextFetchEvent) => Promise<Response>;
