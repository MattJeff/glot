// Next.js (16+ `proxy.ts`, ex-middleware; Node or Vercel Edge): locale URLs for on-demand locales.
//   // proxy.ts
//   import { glotProxy } from "@glot/edge/next";
//   export const proxy = glotProxy({ source: "en", locales: ["de", "it"] });
//   export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
// `/de/x` renders `/x` with the request header `x-glot-locale: de`: read it in `getRequestConfig` and pass it to
// `createOnDemandLoader` (@glot/runtime/ondemand). Unprefixed HTML navigations follow cookie / Accept-Language.
// `edge: { server, project, key? }` = glot Edge: `/de/x` is the app's own `/x` fetched by the proxy (header
// `x-glot-edge-origin: 1`, so it is not routed again) and its rendered HTML translated from the TM (see edge.ts):
// no keys needed, fail-open. `edge.origin` = where to fetch the app (default: the request's origin).
// With `edge`, add "/__glot/:path*" to the matcher (the client patcher and its TM are served there, same origin).
// `experiments: { server, project, key }` adds segmented copy experiments: the arm is decided here and passed as
// `x-glot-xp` (read by `withExperiments` in `getRequestConfig`), so the first HTML already has the variant.
import { NextResponse, type NextFetchEvent, type NextRequest } from "next/server.js"; // .js: plain-Node ESM resolution (tests); bundlers accept both
import { route, type RouteOptions } from "./route.ts";
import { forwardHeaders, glotExperiments, type ExperimentsOptions } from "./experiments.ts";
import { createEdge, type EdgeOptions } from "./edge.ts";
import { translate } from "./translate.ts";

export type EdgeProxyOptions = Omit<EdgeOptions, "source" | "translate"> & { origin?: string; translate?: EdgeOptions["translate"] };

export function glotProxy(o: RouteOptions & { experiments?: ExperimentsOptions; edge?: EdgeProxyOptions }) {
  const xp = o.experiments && glotExperiments(o.experiments);
  const edge = o.edge && createEdge({ translate, locales: o.locales, ...o.edge, source: o.source });
  return async (req: NextRequest, event?: NextFetchEvent) => {
    if (edge && req.headers.get("x-glot-edge-origin") === "1") return NextResponse.next(); // our own origin fetch
    if (edge && req.nextUrl.pathname.startsWith("/__glot/")) {
      const own = await edge.own(req, event && ((p) => event.waitUntil(p)));
      if (own) return own;
    }
    const r = route(req.nextUrl, req.headers, o);
    if ("redirect" in r) {
      const res = NextResponse.redirect(new URL(r.redirect, req.url));
      res.headers.set("vary", "Accept-Language, Cookie");
      return res;
    }
    const d = xp && (await xp(req, r.locale, event && ((p) => event.waitUntil(p))));
    const headers = d ? forwardHeaders(req.headers, d) : new Headers(req.headers);
    headers.set("x-glot-locale", r.locale);
    if (edge && r.locale !== o.source && (req.method === "GET" || req.method === "HEAD")) {
      const originUrl = new URL(r.path + req.nextUrl.search, o.edge!.origin ?? req.nextUrl.origin);
      const fwd = new Headers(headers);
      fwd.set("x-glot-edge-origin", "1");
      fwd.delete("accept-encoding"); // we read the body
      const res = await edge(req, r.locale, originUrl, (u) => fetch(u, { headers: fwd, redirect: "manual" }), event && ((p) => event.waitUntil(p)));
      if (d?.header) res.headers.set("cache-control", "private, no-store");
      for (const c of d?.cookies ?? []) res.headers.append("set-cookie", c);
      return res;
    }
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
