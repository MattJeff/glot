// Next.js (16+ `proxy.ts`, ex-middleware; Node or Vercel Edge): locale URLs for on-demand locales.
//   // proxy.ts
//   import { glotProxy } from "@glot/edge/next";
//   export const proxy = glotProxy({ source: "en", locales: ["de", "it"] });
//   export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
// `/de/x` renders `/x` with the request header `x-glot-locale: de`: read it in `getRequestConfig` and pass it to
// `createOnDemandLoader` (@glot/runtime/ondemand). Unprefixed HTML navigations follow cookie / Accept-Language.
// `experiments: { server, project, key }` adds segmented copy experiments: the arm is decided here and passed as
// `x-glot-xp` (read by `withExperiments` in `getRequestConfig`), so the first HTML already has the variant.
import { NextResponse, type NextFetchEvent, type NextRequest } from "next/server.js"; // .js: plain-Node ESM resolution (tests); bundlers accept both
import { route, type RouteOptions } from "./route.ts";
import { forwardHeaders, glotExperiments, type ExperimentsOptions } from "./experiments.ts";

export function glotProxy(o: RouteOptions & { experiments?: ExperimentsOptions }) {
  const xp = o.experiments && glotExperiments(o.experiments);
  return async (req: NextRequest, event?: NextFetchEvent) => {
    const r = route(req.nextUrl, req.headers, o);
    if ("redirect" in r) {
      const res = NextResponse.redirect(new URL(r.redirect, req.url));
      res.headers.set("vary", "Accept-Language, Cookie");
      return res;
    }
    const d = xp && (await xp(req, r.locale, event && ((p) => event.waitUntil(p))));
    const headers = d ? forwardHeaders(req.headers, d) : new Headers(req.headers);
    headers.set("x-glot-locale", r.locale);
    let res: NextResponse;
    if (r.path === req.nextUrl.pathname) res = NextResponse.next({ request: { headers } });
    else {
      const url = req.nextUrl.clone();
      url.pathname = r.path;
      res = NextResponse.rewrite(url, { request: { headers } });
    }
    if (d?.header) res.headers.set("cache-control", "private, no-store"); // never share a variant through a cache
    for (const c of d?.cookies ?? []) res.headers.append("set-cookie", c);
    return res;
  };
}
