import { NextResponse, type NextRequest } from "next/server";
import { type RouteOptions } from "./route.js";
export declare function glotProxy(o: RouteOptions): (req: NextRequest) => NextResponse<unknown>;
