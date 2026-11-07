// src/edge.ts
function lookup(tm, hash, context) {
  const e = tm.t[hash];
  return typeof e === "string" ? e : e?.[context] ?? e?.["*"];
}
var SESSION_COOKIE = /(?:^|;\s*)[^=;]*(sess|auth|token|sid|jwt|login|logged|user|account)[^=;]*=/i;
var HOP = ["content-length", "content-encoding", "transfer-encoding", "connection", "keep-alive", "etag"];
function createEdge(o) {
  const base = o.server.replace(/\/+$/, "");
  const f = o.fetch ?? ((...a) => globalThis.fetch(...a));
  const now = o.now ?? Date.now;
  const timeout = o.timeout ?? 300;
  const statusTtl = o.statusTtl ?? 2e4;
  const maxHtml = o.maxHtml ?? 3e6;
  const proj = encodeURIComponent(o.project);
  let status;
  let statusRun;
  const tms = /* @__PURE__ */ new Map();
  const tmRun = /* @__PURE__ */ new Map();
  let restored;
  const seen = /* @__PURE__ */ new Set();
  let served = 0, flushedAt = now(), quotaUntil = 0;
  const count = (wait) => {
    served++;
    if (!o.key || !wait || served < 100 && now() - flushedAt < 3e4) return;
    const pages = served;
    served = 0;
    flushedAt = now();
    wait(
      f(`${base}/edge/v1/${proj}/usage`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ key: o.key, pages }) }).then((r) => void (r.status === 402 && (quotaUntil = now() + 3e5))).catch(() => {
      })
    );
  };
  const get = async (url, ms) => {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), ms);
    try {
      return await f(url, { signal: ctl.signal, headers: { accept: "application/json" } });
    } finally {
      clearTimeout(t);
    }
  };
  const restore = () => restored ??= (async () => {
    if (!o.store) return;
    try {
      const s = await o.store.get(`status`);
      if (s && !status) status = { at: 0, v: JSON.parse(s) };
    } catch {
    }
  })();
  const refreshStatus = () => statusRun ??= (async () => {
    try {
      const res = await get(`${base}/edge/v1/${proj}/status`, 5e3);
      if (!res.ok) throw new Error(String(res.status));
      const v = await res.json();
      const ready = await get(`${base}/cdn/${proj}/locales.json`, 5e3).then((r) => r.ok ? r.json() : void 0).then((m) => m?.locales?.filter((l) => l.ready !== false).map((l) => l.code)).catch(() => void 0);
      status = { at: now(), v: { enabled: v.enabled !== false, off: v.off ?? [], versions: v.versions ?? {}, selectors: v.selectors, ready: ready ?? status?.v.ready } };
      void o.store?.put("status", JSON.stringify(status.v)).catch(() => {
      });
      await Promise.all([...tms.values()].filter((t) => (status.v.versions[t.locale] ?? 0) > t.v).map((t) => loadTm(t.locale, status.v.versions[t.locale])));
      return status.v;
    } catch {
      if (status) status.at = now() - statusTtl / 2;
      return status?.v;
    } finally {
      statusRun = void 0;
    }
  })();
  const loadTm = (locale, want) => {
    const key = `${locale}@${want}`;
    const running = tmRun.get(key);
    if (running) return running;
    const p = (async () => {
      const prev = tms.get(locale) ?? await fromStore(locale);
      if (prev && prev.v >= want) return prev;
      try {
        const res = await get(`${base}/edge/v1/${proj}/${encodeURIComponent(locale)}/tm`, 1e4);
        if (!res.ok) throw new Error(String(res.status));
        const body = await res.json();
        if (typeof body?.t !== "object" || body.t === null) throw new Error("bad tm");
        const tm = Object.freeze({ v: Number(body.v) || 0, locale, t: Object.freeze(body.t) });
        if (!prev || tm.v >= prev.v) {
          tms.set(locale, tm);
          void o.store?.put(`tm:${locale}`, JSON.stringify(tm)).catch(() => {
          });
        }
        return tms.get(locale);
      } catch {
        return prev;
      } finally {
        tmRun.delete(key);
      }
    })();
    tmRun.set(key, p);
    return p;
  };
  const fromStore = async (locale) => {
    try {
      const s = await o.store?.get(`tm:${locale}`);
      if (!s) return void 0;
      const tm = JSON.parse(s);
      if (!tms.has(locale)) tms.set(locale, tm);
      return tms.get(locale);
    } catch {
      return void 0;
    }
  };
  async function snapshot(locale, host, wait) {
    await restore();
    const fresh = status && now() - status.at < statusTtl;
    let st = status?.v;
    if (!fresh) {
      const p2 = refreshStatus();
      if (st) wait?.(p2);
      else {
        wait?.(p2);
        st = await race(p2, timeout);
      }
    }
    if (st && (!st.enabled || st.off.includes(host))) return "off";
    const want = st?.versions[locale] ?? 0;
    const have = tms.get(locale) ?? await fromStore(locale);
    if (have && have.v >= want) return have;
    const p = loadTm(locale, want);
    if (have) {
      wait?.(p);
      return have;
    }
    wait?.(p);
    return race(p, timeout);
  }
  function report(locale, out, url, wait) {
    if (!o.key || !wait) return;
    const path = url.pathname;
    if (seen.size > 2e4) seen.clear();
    const segments = out.missing.filter((m) => m.discoverable !== false && !seen.has(`${locale}|${m.hash}|${m.role ?? ""}`)).slice(0, 200).map((m) => (seen.add(`${locale}|${m.hash}|${m.role ?? ""}`), { hash: m.hash, text: m.text, role: m.role ?? "", url: path }));
    const pageKey2 = `page|${locale}|${path}`;
    const pages = out.seen && !seen.has(pageKey2) ? [{ url: path, seen: out.seen.slice(0, 500) }] : [];
    if (pages.length) seen.add(pageKey2);
    if (!segments.length && !pages.length) return;
    wait(
      f(`${base}/edge/v1/${proj}/missing`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: o.key, locale, auth: false, segments, pages })
      }).catch(() => {
      })
    );
  }
  let patcher;
  async function own(req, wait) {
    const u = new URL(req.url);
    if (u.pathname === "/__glot/tm.json") {
      const l = u.searchParams.get("locale") ?? "";
      const tm = l && l !== o.source ? await snapshot(l, u.hostname, wait) : void 0;
      if (!tm || tm === "off") return new Response("{}", { status: 404, headers: { "content-type": "application/json", "cache-control": "max-age=30" } });
      const since = Number(u.searchParams.get("since") ?? -1);
      if (tm.v <= since) return new Response(null, { status: 204, headers: { "cache-control": "max-age=10" } });
      const skip = status?.v.selectors?.exclude;
      return new Response(JSON.stringify(skip?.length ? { ...tm, skip } : tm), { headers: { "content-type": "application/json; charset=utf-8", "cache-control": "public, max-age=30, stale-if-error=86400" } });
    }
    if (u.pathname === "/__glot/missing") return missing(req, wait);
    if (u.pathname === "/__glot/patcher.js") {
      if (!patcher || now() - patcher.at > 6e5) {
        const js = await get(`${base}/edge/patcher.js`, 3e3).then((r) => r.ok ? r.text() : Promise.reject(), () => void 0).catch(() => void 0);
        patcher = { at: now(), js: js ?? patcher?.js ?? "" };
      }
      return new Response(patcher.js, { headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=600" } });
    }
    return void 0;
  }
  async function missing(req, wait) {
    const done = new Response(null, { status: 204 });
    if (!o.key || req.method !== "POST") return done;
    let b;
    try {
      const raw = await req.text();
      if (raw.length > 65536) return done;
      b = JSON.parse(raw);
    } catch {
      return done;
    }
    const str = (x, max) => typeof x === "string" && x.length <= max ? x : void 0;
    const locale = str(b.locale, 35);
    if (!locale || !/^[A-Za-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(locale) || locale === o.source || !Array.isArray(b.segments)) return done;
    const segments = b.segments.slice(0, 200).flatMap((x) => {
      const hash = str(x?.hash, 64), text = str(x?.text, 2e3), role = str(x?.role, 64) ?? "";
      const path = str(x?.url, 2e3)?.split(/[?#]/)[0];
      if (!hash || !text || !path?.startsWith("/")) return [];
      return [{ hash, text, role, url: path.replace(new RegExp(`^/${locale}(?=/|$)`, "i"), "") || "/" }];
    });
    if (!segments.length) return done;
    const p = f(`${base}/edge/v1/${proj}/missing`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key: o.key, locale, auth: !anonymous(req.headers), segments })
    }).catch(() => {
    });
    if (wait) wait(p);
    else await p;
    return done;
  }
  return Object.assign(serve, { own });
  async function serve(req, locale, originUrl, origin, wait) {
    const url = new URL(req.url);
    const t0 = now();
    const translatable = locale !== o.source && (req.method === "GET" || req.method === "HEAD");
    const early = translatable && !o.pageCache ? settle(origin(originUrl)) : void 0;
    const tmP = translatable ? snapshot(locale, url.hostname, wait).catch(() => void 0) : Promise.resolve(void 0);
    const anon = anonymous(req.headers);
    const tm = await tmP;
    const tGlot = now() - t0;
    const cacheKey = tm && tm !== "off" && anon && o.pageCache ? pageKey(originUrl, url.hostname, tm) : void 0;
    if (cacheKey) {
      const hit = await o.pageCache.match(cacheKey).catch(() => void 0);
      if (hit) return count(wait), timing(new Response(hit.body, hit), hit.headers.get("x-glot-coverage")?.startsWith("0/") ? "source" : "translated", tGlot, 0, "hit");
    }
    const got = await (early ?? settle(origin(originUrl)));
    if (!("ok" in got)) return new Response("Bad gateway", { status: 502, headers: { "x-glot-edge": "origin-error" } });
    const page = got.ok;
    const tOrigin = got.ms;
    if (!translatable) return timing(page, "passthrough", 0, tOrigin);
    if (tm === "off") return timing(sourceUnderLocale(page), "off", tGlot, tOrigin);
    if (quotaUntil > now()) return timing(sourceUnderLocale(page), "quota", tGlot, tOrigin);
    const type = page.headers.get("content-type") ?? "";
    const len = Number(page.headers.get("content-length") ?? 0);
    if (!type.includes("text/html") || page.status !== 200 || len > maxHtml || req.method === "HEAD" || !page.body)
      return timing(page, "passthrough", tGlot, tOrigin);
    if (!tm) return timing(sourceUnderLocale(page), "fail-open", tGlot, tOrigin);
    let html;
    try {
      html = await page.text();
    } catch {
      return new Response("Bad gateway", { status: 502, headers: { "x-glot-edge": "origin-error" } });
    }
    if (html.length > maxHtml) return timing(sourceUnderLocale(new Response(html, page)), "passthrough", tGlot, tOrigin);
    const tT = now();
    let out;
    try {
      const ready = status?.v.ready;
      const locales = (o.locales ?? []).filter((l) => l === locale || !ready || ready.includes(l));
      if (!locales.includes(locale)) locales.push(locale);
      const ctx = { url, path: originUrl.pathname, locale, source: o.source, locales, skip: status?.v.selectors?.exclude ?? [], discover: !!o.key && anon };
      out = o.translate(html, tm, ctx);
    } catch {
      return timing(sourceUnderLocale(new Response(html, page)), "fail-open", tGlot + now() - tT, tOrigin);
    }
    if (anon) report(locale, out, originUrl, wait);
    const headers = new Headers(page.headers);
    for (const h of HOP) headers.delete(h);
    headers.set("content-language", locale);
    headers.set("x-glot-version", `${locale}@${tm.v}`);
    headers.set("x-glot-coverage", `${out.translated}/${out.total}`);
    const res = new Response(out.html, { status: page.status, statusText: page.statusText, headers });
    const ttl = cacheKey ? cacheTtl(page.headers, o.pageTtl ?? 0) : 0;
    if (cacheKey && ttl > 0) {
      const copy = res.clone();
      copy.headers.set("cache-control", `public, max-age=${ttl}`);
      wait?.(o.pageCache.put(cacheKey, copy).catch(() => {
      }));
    }
    count(wait);
    return timing(res, out.translated ? "translated" : "source", tGlot + now() - tT, tOrigin, "miss");
  }
  ;
}
var anonymous = (h) => !h.has("authorization") && !SESSION_COOKIE.test(h.get("cookie") ?? "");
function pageKey(u, host, tm) {
  const k = new URL(`https://glot-edge.invalid/${host}${u.pathname}`);
  k.search = u.search;
  k.searchParams.set("__glot", `${tm.locale}.${tm.v}`);
  return k.toString();
}
function cacheTtl(h, fallback) {
  if (h.has("set-cookie")) return 0;
  const cc = (h.get("cache-control") ?? "").toLowerCase();
  if (/private|no-store|no-cache/.test(cc)) return 0;
  const m = /s-maxage=(\d+)/.exec(cc) ?? /max-age=(\d+)/.exec(cc);
  return m ? Number(m[1]) : fallback;
}
function sourceUnderLocale(page) {
  const res = new Response(page.body, page);
  res.headers.set("cache-control", "no-store");
  return res;
}
function timing(res, state, glot, origin, cache) {
  const r = new Response(res.body, res);
  r.headers.set("x-glot-edge", cache === "hit" ? `${state}; cache=hit` : state);
  r.headers.append("server-timing", `glot;dur=${glot.toFixed(1)};desc="${state}${cache ? ` ${cache}` : ""}", origin;dur=${origin.toFixed(1)}`);
  return r;
}
function settle(p) {
  const t = Date.now();
  return p.then((ok) => ({ ok, ms: Date.now() - t }), (err) => ({ err, ms: Date.now() - t }));
}
function race(p, ms) {
  return new Promise((resolve) => {
    const t = setTimeout(() => resolve(void 0), ms);
    p.then(
      (v) => (clearTimeout(t), resolve(v)),
      () => (clearTimeout(t), resolve(void 0))
    );
  });
}
export {
  SESSION_COOKIE,
  cacheTtl,
  createEdge,
  lookup
};
