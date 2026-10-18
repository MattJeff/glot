// Next.js (16+ `proxy.ts`, ex-middleware; Node or Vercel Edge): locale URLs for on-demand locales.
//   // proxy.ts
//   import { glotProxy } from "@glot/edge/next";
//   export const proxy = glotProxy({ source: "en", locales: ["de", "it"] });
//   export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
// `/de/x` renders `/x` with the request header `x-glot-locale: de`: read it in `getRequestConfig` and pass it to
// `createOnDemandLoader` (@glot/runtime/ondemand). Unprefixed HTML navigations follow cookie / Accept-Language.
import { NextResponse, type NextRequest } from "next/server";
import { route, type RouteOptions } from "./route.ts";

export function glotProxy(o: RouteOptions) {
  return (req: NextRequest) => {
    const r = route(req.nextUrl, req.headers, o);
    if ("redirect" in r) {
      const res = NextResponse.redirect(new URL(r.redirect, req.url));
      res.headers.set("vary", "Accept-Language, Cookie");
      return res;
    }
    const headers = new Headers(req.headers);
    headers.set("x-glot-locale", r.locale);
    if (r.path === req.nextUrl.pathname) return NextResponse.next({ request: { headers } });
    const url = req.nextUrl.clone();
    url.pathname = r.path;
    return NextResponse.rewrite(url, { request: { headers } });
  };
}
