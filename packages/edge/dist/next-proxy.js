// src/next-proxy.ts
import { NextResponse } from "next/server.js";

// ../runtime/dist/ondemand.js
function b(e) {
  return (e ?? "").split(",").map((t) => {
    let [s, ...a] = t.trim().split(";"), r = a.map((o) => o.trim()).find((o) => o.startsWith("q="));
    return { tag: s.trim(), q: r ? Number(r.slice(2)) : 1 };
  }).filter((t) => t.tag && t.tag !== "*" && t.q > 0).sort((t, s) => s.q - t.q).map((t) => t.tag);
}

// src/route.ts
function route(url, headers, { source: source2, locales }) {
  const [, first = "", ...rest] = url.pathname.split("/");
  const ok = (l) => !!l && l !== source2 && locales.includes(l);
  if (ok(first)) return { locale: first, path: "/" + rest.join("/") };
  const cookie2 = /(?:^|;\s*)glot_locale=([^;]+)/.exec(headers.get("cookie") ?? "")?.[1];
  const wanted = cookie2 ?? b(headers.get("accept-language"))[0]?.toLowerCase().split("-")[0];
  if (ok(wanted) && (headers.get("accept") ?? "").includes("text/html")) return { redirect: `/${wanted}${url.pathname}${url.search}` };
  return { locale: source2, path: url.pathname };
}

// src/experiments.ts
var NONE = { assignments: [], overlay: {}, exposures: [], header: null, cookies: [] };
var BOT = /bot|crawl|spider|slurp|preview|facebookexternalhit|embedly|whatsapp|headless|lighthouse|pingdom|curl|wget|python|go-http|java\/|okhttp|axios|node-fetch/i;
var SEARCH = /(^|\.)(google|bing|yahoo|duckduckgo|baidu|yandex|ecosia|qwant|startpage|naver|seznam)\./;
var SOCIAL = /(^|\.)(facebook|fb|instagram|t|twitter|x|linkedin|lnkd|reddit|pinterest|tiktok|youtube|threads|bsky|mastodon)\.|^(t\.co|lnkd\.in)$/;
var EMAIL = /(^|\.)(mail|outlook|gmail)\./;
function isDocument(req, url = new URL(req.url)) {
  const h = req.headers;
  if (/prefetch|prerender/.test(`${h.get("sec-purpose") ?? ""}${h.get("purpose") ?? ""}${h.get("x-purpose") ?? ""}`)) return false;
  if (h.has("next-router-prefetch") || h.has("rsc") || h.has("next-router-state-tree") || url.searchParams.has("_rsc")) return false;
  const dest = h.get("sec-fetch-dest");
  return dest ? dest === "document" : (h.get("accept") ?? "").includes("text/html");
}
var isBot = (ua) => !ua || BOT.test(ua);
var cookie = (h, name) => new RegExp(`(?:^|;\\s*)${name}=([^;]*)`).exec(h.get("cookie") ?? "")?.[1];
function context(req, o, url = new URL(req.url)) {
  const h = req.headers;
  const ctx = { ...o.attrs, locale: o.locale };
  const country = (h.get("x-vercel-ip-country") ?? h.get("cf-ipcountry") ?? h.get("cloudfront-viewer-country") ?? h.get("fastly-geo-country") ?? "").toUpperCase();
  if (/^[A-Z]{2}$/.test(country) && country !== "XX") ctx.country = country;
  const region = h.get("x-vercel-ip-country-region") ?? h.get("cf-region-code") ?? h.get("cloudfront-viewer-country-region");
  if (region) ctx.region = region.toUpperCase();
  const ua = h.get("user-agent") ?? "";
  ctx.device = /ipad|tablet|kindle|silk|android(?!.*mobile)/i.test(ua) ? "tablet" : h.get("sec-ch-ua-mobile") === "?1" || /mobi|iphone|ipod|android/i.test(ua) ? "mobile" : "desktop";
  ctx.source = source(url, h.get("referer"));
  ctx.visitor = o.consent ? cookie(h, "glot_seen") ? "returning" : "new" : "unknown";
  return ctx;
}
function source(url, referer) {
  const q = url.searchParams;
  const medium = (q.get("utm_medium") ?? "").toLowerCase();
  if (/cpc|ppc|paid|display|cpm|banner|ads?$/.test(medium) || q.has("gclid") || q.has("fbclid") || q.has("msclkid")) return "paid";
  if (medium.includes("email") || medium === "newsletter") return "email";
  if (medium.includes("social")) return "social";
  if (medium === "organic") return "search";
  let host = (q.get("utm_source") ?? "").toLowerCase();
  if (host) host += ".";
  else if (referer) {
    try {
      host = new URL(referer).hostname.replace(/^www\./, "");
    } catch {
    }
    if (host === url.hostname.replace(/^www\./, "")) return "direct";
  }
  if (!host) return "direct";
  return SEARCH.test(host) ? "search" : SOCIAL.test(host) ? "social" : EMAIL.test(host) ? "email" : "referral";
}
var matches = (rule, ctx) => rule.all.every((p) => {
  const v = ctx[p.attr];
  if (p.not_in) return v === void 0 || !p.not_in.includes(v);
  return v !== void 0 && (p.eq !== void 0 ? v === p.eq : !!p.in?.includes(v));
});
function hash(s) {
  let x = 2166136261;
  for (let i = 0; i < s.length; i++) x = Math.imul(x ^ s.charCodeAt(i), 16777619);
  return (x >>> 0) / 4294967296;
}
function pick(weights, u) {
  let total = 0;
  for (const w of weights) total += w > 0 ? w : 0;
  if (!(total > 0)) return -1;
  let acc = 0, last = -1;
  for (let i = 0; i < weights.length; i++) {
    if (!(weights[i] > 0)) continue;
    last = i;
    if (u * total < (acc += weights[i])) return i;
  }
  return last;
}
function decide(req, config, o) {
  const now = o.now ?? Date.now();
  if (!config || config.v !== 1 || !((config.fetched_at ?? Date.parse(config.generated_at)) + config.max_stale * 1e3 > now)) return NONE;
  const h = req.headers;
  if (h.get("sec-gpc") === "1" || h.get("dnt") === "1") return NONE;
  const url = new URL(req.url);
  const salt = o.salt ?? "glot";
  const u = (k2) => o.unit !== void 0 ? hash(`${salt}:${k2}:${o.unit}`) : (o.rand ?? Math.random)();
  const session = config.persistence === "session";
  const saved = /* @__PURE__ */ new Map();
  if (session)
    for (const part of (cookie(h, "glot_xp") ?? "").split("~")) {
      const [id, arm, cell] = part.split(".");
      if (id && cell && /^\d$/.test(arm)) saved.set(id, [Number(arm), cell]);
    }
  const assignments = [];
  const counted = [];
  const overlay = {};
  const k = {};
  const apply = (xp, i) => {
    const v = xp.arms[i].values?.[o.locale];
    if (v) Object.assign(overlay, v);
  };
  const forced = url.searchParams.get("glot_xp");
  if (forced) {
    const [id, arm] = forced.split(".");
    const xp = config.experiments.find((x) => x.id === id);
    const i = xp ? xp.arms.findIndex((a) => a.arm === Number(arm)) : -1;
    if (!xp || i < 0) return NONE;
    apply(xp, i);
    return finish(o.locale, [{ id, cell: "*", arm: xp.arms[i].arm }], overlay, [], []);
  }
  let inHoldout = false;
  if (config.holdout) {
    const { id } = config.holdout;
    const prev = saved.get(id);
    const arm = prev && prev[1] === "*" ? prev[0] : u(id) * 100 < config.holdout.pct ? 0 : 1;
    inHoldout = arm === 0;
    assignments.push({ id, cell: "*", arm });
    if (!(prev && prev[0] === arm && prev[1] === "*")) counted.push({ id, cell: "*", arm });
  }
  if (!inHoldout) {
    const ctx = context(req, o, url);
    for (const xp of config.experiments) {
      if (!xp.locales.includes(o.locale)) continue;
      const prev = saved.get(xp.id);
      let cell = prev && xp.cells.find((c) => c.id === prev[1]);
      let i = cell ? xp.arms.findIndex((a2) => a2.arm === prev[0]) : -1;
      if (!cell || i < 0 || !(cell.weights[i] > 0)) {
        if (u(`${xp.id}:t`) * 100 >= xp.traffic_pct) continue;
        cell = xp.cells.find((c) => {
          const seg = c.id !== "*" && config.segments.find((s) => s.id === c.id);
          return c.id === "*" || !!seg && matches(seg.rule, ctx);
        });
        if (!cell) continue;
        i = pick(cell.weights, u(xp.id));
        if (i < 0) continue;
      }
      const a = { id: xp.id, cell: cell.id, arm: xp.arms[i].arm };
      assignments.push(a);
      const live = cell.count && xp.status === "running";
      if (live && !(prev && prev[0] === a.arm && prev[1] === a.cell)) counted.push(a);
      if (live && xp.keys?.length) k[xp.id] = xp.keys;
      apply(xp, i);
    }
  }
  const cookies = [];
  const secure = url.protocol === "https:" ? "; Secure" : "";
  if (session && assignments.length) {
    const kept = [...assignments];
    for (const [id, [arm, cell]] of saved)
      if (!kept.some((a) => a.id === id) && config.experiments.some((x) => x.id === id)) kept.push({ id, arm, cell });
    const value = kept.map((a) => `${a.id}.${a.arm}.${a.cell}`).join("~");
    if (value !== cookie(h, "glot_xp")) cookies.push(`glot_xp=${value}; Path=/; SameSite=Lax${secure}`);
  }
  if (o.consent && !cookie(h, "glot_seen")) cookies.push(`glot_seen=1; Path=/; Max-Age=31536000; SameSite=Lax${secure}`);
  const count = isDocument(req, url) && !isBot(h.get("user-agent"));
  return finish(o.locale, assignments, overlay, count ? counted : [], cookies, k);
}
function finish(locale, a, o, exposures, cookies, k = {}) {
  const x = Object.keys(k).length ? { v: 1, locale, a, o, k } : { v: 1, locale, a, o };
  return { assignments: a, overlay: o, exposures, cookies, header: a.length ? encode(JSON.stringify(x)) : null };
}
function encode(s) {
  const b2 = new TextEncoder().encode(s);
  let bin = "";
  for (let i = 0; i < b2.length; i += 32768) bin += String.fromCharCode(...b2.subarray(i, i + 32768));
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function createExperimentsLoader({ server, project, ttl = 6e4, timeout = 150, fetch: f = globalThis.fetch }) {
  const url = `${server.replace(/\/+$/, "")}/cdn/${encodeURIComponent(project)}/experiments.json`;
  let hit;
  let inflight;
  const refresh = () => inflight ??= (async () => {
    try {
      const res = await f(url, { headers: hit?.etag ? { "if-none-match": hit.etag } : {}, signal: AbortSignal.timeout(5e3) });
      if (res.status === 304 && hit) {
        hit.at = Date.now();
        if (hit.config) hit.config.fetched_at = hit.at;
      } else if (res.ok) {
        const config = await res.json();
        config.fetched_at = Date.now();
        hit = { at: config.fetched_at, etag: res.headers.get("etag") ?? void 0, config };
      } else hit = { at: Date.now(), config: null };
    } catch {
      if (hit) hit.at = Date.now();
    } finally {
      inflight = void 0;
    }
    return hit?.config ?? null;
  })();
  return async function load(waitUntil) {
    if (hit) {
      if (Date.now() - hit.at >= ttl) {
        const p2 = refresh();
        waitUntil?.(p2);
      }
      return hit.config;
    }
    const p = refresh();
    waitUntil?.(p);
    let timer;
    const late = new Promise((r) => timer = setTimeout(r, timeout, null));
    return Promise.race([p, late]).finally(() => clearTimeout(timer));
  };
}
function glotExperiments(o) {
  const load = createExperimentsLoader(o);
  const endpoint = `${o.server.replace(/\/+$/, "")}/api/insights/${encodeURIComponent(o.key)}/batch`;
  const f = o.fetch ?? globalThis.fetch;
  let queue = [];
  let timer;
  const flush = async () => {
    while (queue.length) {
      const body = JSON.stringify(queue.splice(0, 50));
      await f(endpoint, { method: "POST", body }).catch(() => {
      });
    }
  };
  return async (req, locale, waitUntil) => {
    let d = NONE;
    try {
      d = decide(req, await load(waitUntil), { locale, consent: o.consent?.(req), attrs: o.attrs?.(req) });
    } catch {
    }
    if (d.exposures.length) {
      queue.push({ served: locale, exposures: d.exposures });
      const p = queue.length >= 50 ? flush() : timer ??= new Promise((r) => setTimeout(r, o.batchMs ?? 200)).then(() => (timer = void 0, flush()));
      waitUntil?.(p);
    }
    return d;
  };
}
function forwardHeaders(headers, d) {
  const out = new Headers(headers);
  out.delete("x-glot-xp");
  if (d.header) out.set("x-glot-xp", d.header);
  return out;
}

// src/next-proxy.ts
function glotProxy(o) {
  const xp = o.experiments && glotExperiments(o.experiments);
  return async (req, event) => {
    const r = route(req.nextUrl, req.headers, o);
    if ("redirect" in r) {
      const res2 = NextResponse.redirect(new URL(r.redirect, req.url));
      res2.headers.set("vary", "Accept-Language, Cookie");
      return res2;
    }
    const d = xp && await xp(req, r.locale, event && ((p) => event.waitUntil(p)));
    const headers = d ? forwardHeaders(req.headers, d) : new Headers(req.headers);
    headers.set("x-glot-locale", r.locale);
    let res;
    if (r.path === req.nextUrl.pathname) res = NextResponse.next({ request: { headers } });
    else {
      const url = req.nextUrl.clone();
      url.pathname = r.path;
      res = NextResponse.rewrite(url, { request: { headers } });
    }
    if (d?.header) res.headers.set("cache-control", "private, no-store");
    for (const c of d?.cookies ?? []) res.headers.append("set-cookie", c);
    return res;
  };
}
export {
  glotProxy
};
