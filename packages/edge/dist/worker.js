// ../runtime/dist/ondemand.js
function J({ server: e, project: t, source: s, ttl: a = 6e4, fetch: r = globalThis.fetch }) {
  let o = e.replace(/\/+$/, ""), c = /* @__PURE__ */ new Map(), m = /* @__PURE__ */ new Map(), f = (i) => {
    let u = m.get(i);
    if (u) return u;
    let n = c.get(i), d = (async () => {
      try {
        let g = await r(`${o}/cdn/${encodeURIComponent(t)}/${encodeURIComponent(i)}.json?ondemand=1`, { headers: n?.etag ? { "if-none-match": n.etag } : {} });
        if (g.status === 304 && n) return n.at = Date.now(), n.messages;
        let p = g.status === 200 ? y(s ?? {}, await g.json()) : null, l = (Number(g.headers.get("retry-after") ?? 5) || 0) * 1e3;
        return c.set(i, { at: Date.now(), ttl: p ? a : Math.min(a, l), etag: g.headers.get("etag") ?? void 0, messages: p }), p;
      } catch {
        return n?.messages ?? null;
      } finally {
        m.delete(i);
      }
    })();
    return m.set(i, d), d;
  };
  return async function(u) {
    let n = c.get(u);
    return n ? Date.now() - n.at < n.ttl ? n.messages : n.messages ? (f(u), n.messages) : f(u) : f(u);
  };
}
function b(e) {
  return (e ?? "").split(",").map((t) => {
    let [s, ...a] = t.trim().split(";"), r = a.map((o) => o.trim()).find((o) => o.startsWith("q="));
    return { tag: s.trim(), q: r ? Number(r.slice(2)) : 1 };
  }).filter((t) => t.tag && t.tag !== "*" && t.q > 0).sort((t, s) => s.q - t.q).map((t) => t.tag);
}
function y(e, t) {
  let s = { ...e };
  for (let [a, r] of Object.entries(t)) {
    let o = s[a];
    s[a] = h(o) && h(r) ? y(o, r) : r;
  }
  return s;
}
var h = (e) => typeof e == "object" && e !== null && !Array.isArray(e);

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

// src/worker.ts
var load;
var worker_default = {
  async fetch(req, env) {
    const url = new URL(req.url);
    const source = env.SOURCE_LOCALE ?? "en";
    const r = route(url, req.headers, { source, locales: env.LOCALES.split(",").map((l) => l.trim()) });
    if ("redirect" in r) return new Response(null, { status: 302, headers: { location: r.redirect, vary: "Accept-Language, Cookie" } });
    if (r.locale === source) return fetch(req);
    url.pathname = r.path;
    const page = await fetch(new Request(url, req));
    if (!page.headers.get("content-type")?.includes("text/html")) return page;
    load ??= J({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT });
    const messages = await load(r.locale);
    if (!messages) {
      const res = new Response(page.body, page);
      res.headers.set("cache-control", "no-store");
      return res;
    }
    const lookup = (key) => key.split(".").reduce((o, k) => o?.[k], messages);
    return new HTMLRewriter().on("html", { element: (e) => e.setAttribute("lang", r.locale) }).on("[data-glot]", {
      element(e) {
        const v = lookup(e.getAttribute("data-glot") ?? "");
        if (typeof v === "string") e.setInnerContent(v);
      }
    }).transform(page);
  }
};
export {
  worker_default as default
};
