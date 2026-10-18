import { NextResponse, type NextFetchEvent, type NextRequest } from "next/server.js";
import { type RouteOptions } from "./route.js";
import { type ExperimentsOptions } from "./experiments.js";
export declare function glotProxy(o: RouteOptions & {
    experiments?: ExperimentsOptions;
}): (req: NextRequest, event?: NextFetchEvent) => Promise<NextResponse<unknown>>;
