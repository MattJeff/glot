// src/next-proxy.ts
import { NextResponse } from "next/server";

// ../runtime/dist/ondemand.js
function b(e) {
  return (e ?? "").split(",").map((t) => {
    let [s, ...a] = t.trim().split(";"), r = a.map((o) => o.trim()).find((o) => o.startsWith("q="));
    return { tag: s.trim(), q: r ? Number(r.slice(2)) : 1 };
  }).filter((t) => t.tag && t.tag !== "*" && t.q > 0).sort((t, s) => s.q - t.q).map((t) => t.tag);
}

// src/route.ts
function route(url, headers, { source, locales }) {
  const [, first = "", ...rest] = url.pathname.split("/");
  const ok = (l) => !!l && l !== source && locales.includes(l);
  if (ok(first)) return { locale: first, path: "/" + rest.join("/") };
  const cookie = /(?:^|;\s*)glot_locale=([^;]+)/.exec(headers.get("cookie") ?? "")?.[1];
  const wanted = cookie ?? b(headers.get("accept-language"))[0]?.toLowerCase().split("-")[0];
  if (ok(wanted) && (headers.get("accept") ?? "").includes("text/html")) return { redirect: `/${wanted}${url.pathname}${url.search}` };
  return { locale: source, path: url.pathname };
}

// src/next-proxy.ts
function glotProxy(o) {
  return (req) => {
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
export {
  glotProxy
};
