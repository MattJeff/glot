// src/app-paths.ts
var HOP_COOKIE = "glot_app_hop";
function parseLine(line, def) {
  const l = line.trim();
  if (!l || l.startsWith("#")) return void 0;
  const t = l.split(/\s+/);
  const a = t[0].toLowerCase();
  const [action, p, extra] = a === "redirect" || a === "proxy" ? [a, t[1], t[2]] : [def, t[0], t[1]];
  if (!p?.startsWith("/") || extra !== void 0 || p.length > 200) return void 0;
  return [action, p.toLowerCase()];
}
var unreserved = (c) => c >= 48 && c <= 57 || c >= 65 && c <= 90 || c >= 97 && c <= 122 || c === 45 || c === 46 || c === 95 || c === 126;
var ALLOWED = new Set([..."/!$&'()*+,;=:@"].map((c) => c.charCodeAt(0)));
var hex2 = (v) => "%" + v.toString(16).toUpperCase().padStart(2, "0");
function normalize(path) {
  const b = new TextEncoder().encode(path);
  const hx = (c) => c === void 0 ? NaN : parseInt(String.fromCharCode(c), 16);
  let s = "";
  for (let i = 0; i < b.length; i++) {
    const c = b[i];
    if (c === 37) {
      const h = hx(b[i + 1]), l = hx(b[i + 2]);
      if (!Number.isNaN(h) && !Number.isNaN(l)) {
        const v = h * 16 + l;
        s += unreserved(v) ? String.fromCharCode(v) : hex2(v);
        i += 2;
        continue;
      }
    }
    s += c === 92 ? "/" : unreserved(c) || ALLOWED.has(c) ? String.fromCharCode(c) : hex2(c);
  }
  const trailing = s.endsWith("/") || s.endsWith("/.") || s.endsWith("/..");
  const segs = [];
  for (const seg of s.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") segs.pop();
    else segs.push(seg);
  }
  return "/" + segs.join("/") + (trailing && segs.length ? "/" : "");
}
function wild(p, s) {
  let pi = 0, si = 0, star = -1, mark = 0;
  while (si < s.length) {
    if (pi < p.length && p[pi] === "*") star = pi++, mark = si;
    else if (pi < p.length && p[pi] === s[si]) pi++, si++;
    else if (star >= 0) pi = star + 1, si = ++mark;
    else return false;
  }
  while (pi < p.length && p[pi] === "*") pi++;
  return pi === p.length;
}
function matches(pattern, path) {
  if (pattern === "/") return true;
  if (pattern.includes("*")) return wild(pattern, path);
  if (pattern.endsWith("/")) return path.length > pattern.length && path.startsWith(pattern);
  return path === pattern || path.startsWith(pattern + "/");
}
function classify(rules, def, path) {
  const p = normalize(path).toLowerCase();
  if (p === "/sitemap.xml" || p === "/robots.txt" || p.startsWith("/__glot/")) return void 0;
  for (const l of rules) {
    const r = parseLine(l, def);
    if (r && matches(r[1], p)) return r[0];
  }
  return void 0;
}
function decide(i) {
  const action = classify(i.rules, i.action, i.path);
  if (!action) return void 0;
  if (action === "proxy") return { kind: "proxy" };
  let base;
  try {
    base = new URL(i.origin);
  } catch {
    return { kind: "proxy" };
  }
  const norm = normalize(i.path);
  const hop = norm.replace(/[^A-Za-z0-9/._~%-]/g, "_");
  const bounced = i.cookie.split(";").some((raw) => {
    const kv = raw.trim();
    const at = kv.indexOf("=");
    return at > 0 && kv.slice(0, at).trim() === HOP_COOKIE && kv.slice(at + 1) === hop;
  });
  const target = i.locale_style === "prefix" ? `/${i.locale}${norm}` : norm;
  let q = i.query.split("&").filter(Boolean);
  if (i.locale_style.startsWith("query:")) {
    const param = i.locale_style.slice(6);
    q = q.filter((kv) => kv.split("=")[0] !== param);
    q.push(`${param}=${i.locale}`);
  }
  if (bounced || base.host.toLowerCase() === i.host.toLowerCase() && target === normalize(i.req_path)) return { kind: "proxy" };
  const get = /^(get|head)$/i.test(i.method);
  return {
    kind: "redirect",
    status: get ? 302 : 307,
    location: base.origin + target + (q.length ? "?" + q.join("&") : ""),
    cookie: `${HOP_COOKIE}=${hop}; Max-Age=30; Path=/; HttpOnly; Secure; SameSite=Lax`
  };
}

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
      status = { at: now(), v: { enabled: v.enabled !== false, off: v.off ?? [], versions: v.versions ?? {}, selectors: v.selectors, ready: ready ?? status?.v.ready, app: v.app, auth: v.auth } };
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
  async function appRules(wait) {
    await restore();
    if (!status || now() - status.at >= statusTtl) {
      const p = refreshStatus();
      wait?.(p);
      if (!status) await race(p, timeout);
    }
    return status?.v.app;
  }
  async function isApp(path, wait) {
    const a = await appRules(wait);
    return !!a?.rules?.length && classify(a.rules, a.action ?? "redirect", path) !== void 0;
  }
  async function app(req, locale, path, wait) {
    const a = await appRules(wait);
    if (!a?.rules?.length) return void 0;
    const u = new URL(req.url);
    const d = decide({ rules: a.rules, action: a.action ?? "redirect", locale_style: a.locale ?? "none", locale, origin: u.origin, host: u.host, req_path: u.pathname, path, query: u.search.slice(1), method: req.method, cookie: req.headers.get("cookie") ?? "" });
    if (!d || d.kind === "proxy") return d && "proxy";
    return new Response(null, { status: d.status, headers: { location: d.location, "set-cookie": d.cookie, "cache-control": "no-store", "x-glot-edge": "app-redirect" } });
  }
  async function bundle(req, locale, origin, wait) {
    const t0 = now();
    const js = (r) => r.status === 200 && /javascript|ecmascript/.test(r.headers.get("content-type") ?? "");
    const revalidate = (r) => {
      const res = new Response(r.body, r);
      res.headers.set("cache-control", "public, no-cache");
      res.headers.delete("expires");
      return res;
    };
    const tm = locale === o.source || req.method !== "GET" ? void 0 : await snapshot(locale, new URL(req.url).hostname, wait).catch(() => void 0);
    if (!tm || tm === "off" || !o.key) {
      const r = await origin(req);
      if (!js(r) && r.status !== 304) return r;
      if (locale === o.source || tm === "off" || r.status === 304) return revalidate(r);
      const res = revalidate(r);
      for (const h of ["etag", "last-modified"]) res.headers.delete(h);
      res.headers.set("cache-control", "no-store");
      return timing(res, "fail-open", now() - t0, 0);
    }
    const key = `https://glot-edge.invalid/_bundle/${locale}.${tm.v}${new URL(req.url).pathname}${new URL(req.url).search}`;
    const fresh = (r) => req.headers.get("if-none-match") === r.headers.get("etag") && r.headers.get("etag") ? new Response(null, { status: 304, headers: r.headers }) : r;
    const hit = await o.pageCache?.match(key).catch(() => void 0);
    if (hit) return timing(fresh(revalidate(hit)), "translated", now() - t0, 0, "hit");
    const up = new Request(req);
    up.headers.delete("if-none-match");
    up.headers.delete("if-modified-since");
    const page = await origin(up);
    if (!js(page)) return page;
    const tOrigin = now() - t0;
    const body = await page.arrayBuffer();
    const failOpen = () => {
      const r = new Response(body, page);
      for (const h of ["etag", "last-modified", "expires"]) r.headers.delete(h);
      r.headers.set("cache-control", "no-store");
      return timing(r, "fail-open", now() - t0 - tOrigin, tOrigin);
    };
    if (body.byteLength > 4 * 1024 * 1024) return failOpen();
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), Math.max(timeout, 2e3));
    try {
      const g = await f(`${base}/edge/v1/${proj}/${encodeURIComponent(locale)}/bundle`, { method: "POST", signal: ctl.signal, headers: { "content-type": "text/javascript", "x-glot-key": o.key }, body });
      if (!g.ok) return failOpen();
      const headers = new Headers(page.headers);
      for (const h of [...HOP, "last-modified", "expires"]) headers.delete(h);
      headers.set("cache-control", "public, no-cache");
      for (const h of ["etag", "x-glot-bundle", "x-glot-version"]) {
        const v = g.headers.get(h);
        if (v) headers.set(h, v);
      }
      const out = await g.arrayBuffer();
      if (o.pageCache) {
        const copy = new Response(out, { headers });
        copy.headers.set("cache-control", "public, max-age=31536000");
        wait?.(o.pageCache.put(key, copy).catch(() => {
        }));
      }
      return timing(fresh(new Response(out, { status: 200, headers })), "translated", now() - t0 - tOrigin, tOrigin, "miss");
    } catch {
      return failOpen();
    } finally {
      clearTimeout(timer);
    }
  }
  return Object.assign(serve, { own, app, isApp, bundle });
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
    if (!anon && tm !== "off" && (status?.v.auth?.pages ?? "server") !== "server") return timing(personal(new Response(page.body, page)), "auth", tGlot, tOrigin);
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
    if (!anon) personal(res);
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
function personal(res) {
  res.headers.set("cache-control", "private, no-cache");
  res.headers.append("vary", "Cookie");
  return res;
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
