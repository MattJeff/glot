// src/express.ts
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { join } from "node:path";

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

// src/edge.ts
function lookup(tm, hash, context) {
  const e = tm.t[hash];
  return typeof e === "string" ? e : e?.[context] ?? e?.["*"];
}
var SESSION_COOKIE = /(?:^|;\s*)[^=;]*(sess|auth|token|sid|jwt|login|logged|user|account)[^=;]*=/i;
var HOP = ["content-length", "content-encoding", "transfer-encoding", "connection", "keep-alive", "etag"];
function createEdge(o) {
  const base2 = o.server.replace(/\/+$/, "");
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
      f(`${base2}/edge/v1/${proj}/usage`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ key: o.key, pages }) }).then((r) => void (r.status === 402 && (quotaUntil = now() + 3e5))).catch(() => {
      })
    );
  };
  const get2 = async (url, ms) => {
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
      const res = await get2(`${base2}/edge/v1/${proj}/status`, 5e3);
      if (!res.ok) throw new Error(String(res.status));
      const v = await res.json();
      const ready = await get2(`${base2}/cdn/${proj}/locales.json`, 5e3).then((r) => r.ok ? r.json() : void 0).then((m) => m?.locales?.filter((l) => l.ready !== false).map((l) => l.code)).catch(() => void 0);
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
        const res = await get2(`${base2}/edge/v1/${proj}/${encodeURIComponent(locale)}/tm`, 1e4);
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
      f(`${base2}/edge/v1/${proj}/missing`, {
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
        const js = await get2(`${base2}/edge/patcher.js`, 3e3).then((r) => r.ok ? r.text() : Promise.reject(), () => void 0).catch(() => void 0);
        patcher = { at: now(), js: js ?? patcher?.js ?? "" };
      }
      return new Response(patcher.js, { headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=600" } });
    }
    return void 0;
  }
  async function missing(req, wait) {
    const done = new Response(null, { status: 204 });
    if (!o.key || req.method !== "POST") return done;
    let b2;
    try {
      const raw = await req.text();
      if (raw.length > 65536) return done;
      b2 = JSON.parse(raw);
    } catch {
      return done;
    }
    const str = (x, max) => typeof x === "string" && x.length <= max ? x : void 0;
    const locale = str(b2.locale, 35);
    if (!locale || !/^[A-Za-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(locale) || locale === o.source || !Array.isArray(b2.segments)) return done;
    const segments = b2.segments.slice(0, 200).flatMap((x) => {
      const hash = str(x?.hash, 64), text = str(x?.text, 2e3), role = str(x?.role, 64) ?? "";
      const path = str(x?.url, 2e3)?.split(/[?#]/)[0];
      if (!hash || !text || !path?.startsWith("/")) return [];
      return [{ hash, text, role, url: path.replace(new RegExp(`^/${locale}(?=/|$)`, "i"), "") || "/" }];
    });
    if (!segments.length) return done;
    const p = f(`${base2}/edge/v1/${proj}/missing`, {
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

// src/segment.ts
var isPii = (k) => !["number", "url", "code", "public"].includes(k);
var HTML_WS = " 	\n\r\f";
var isWs = (c) => c !== "" && HTML_WS.includes(c);
var isSep = (c) => isWs(c) || c === "\xA0" || c === "\u202F";
var allWs = (s) => [...s].every(isWs);
var ALPHA = /\p{Alphabetic}/u;
var ALNUM = /[\p{Alphabetic}\p{N}]/u;
var asciiLower = (s) => s.replace(/[A-Z]+/g, (m) => m.toLowerCase());
var eqIgnoreAscii = (a, b2) => asciiLower(a) === asciiLower(b2);
var isDigit = (c) => c !== void 0 && c >= "0" && c <= "9";
var isAsciiAlpha = (c) => c !== void 0 && /^[A-Za-z]$/.test(c);
function collapseWs(s) {
  let out = "";
  let ws = false;
  for (const c of s) {
    if (isWs(c)) ws = true;
    else {
      if (ws && out) out += " ";
      ws = false;
      out += c;
    }
  }
  return out;
}
var NAMED = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: "\xA0",
  copy: "\xA9",
  reg: "\xAE",
  trade: "\u2122",
  hellip: "\u2026",
  mdash: "\u2014",
  ndash: "\u2013",
  laquo: "\xAB",
  raquo: "\xBB",
  lsquo: "\u2018",
  rsquo: "\u2019",
  ldquo: "\u201C",
  rdquo: "\u201D",
  euro: "\u20AC",
  middot: "\xB7",
  bull: "\u2022",
  times: "\xD7"
};
function decodeEntities(s) {
  if (!s.includes("&")) return s;
  return s.replace(/&([A-Za-z0-9#]{1,10});/g, (m, name) => {
    let code;
    if (/^#[xX][0-9a-fA-F]{1,6}$/.test(name)) code = parseInt(name.slice(2), 16);
    else if (/^#[0-9]{1,7}$/.test(name)) code = parseInt(name.slice(1), 10);
    else if (name.startsWith("#")) return m;
    else return Object.hasOwn(NAMED, name) ? NAMED[name] : m;
    return code > 1114111 || code >= 55296 && code <= 57343 ? m : String.fromCodePoint(code);
  });
}
var escapeText = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
var escapeAttr = (s) => escapeText(s).replace(/"/g, "&quot;");
var hasLetter = (s) => ALPHA.test(s);
var isEmail = (w) => {
  const at = w.indexOf("@");
  if (at < 0) return false;
  const local = w.slice(0, at), domain = w.slice(at + 1);
  const tld = domain.split(".").pop() ?? "";
  return local !== "" && /^[A-Za-z0-9._%+-]+$/.test(local) && domain.includes(".") && /^[A-Za-z0-9.-]+$/.test(domain) && tld.length >= 2 && /^[A-Za-z]+$/.test(tld);
};
var isUuid = (w) => /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/.test(w);
var isIban = (w) => /^[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}$/.test(w);
var isToken = (w) => /[0-9]/.test(w) && /[A-Za-z]/.test(w) && /^[A-Za-z0-9_\-.=+/]+$/.test(w) && (w.length >= 20 || w.length >= 16 && /^[0-9a-fA-F]+$/.test(w));
var isCode = (w) => w.endsWith("()") || w.includes("::") || /[A-Za-z0-9]_[A-Za-z0-9]/.test(w);
var isUrl = (w) => {
  const l = asciiLower(w);
  return l.startsWith("http://") || l.startsWith("https://") || l.startsWith("www.") || l.includes("://");
};
function wordKind(w) {
  if (isUrl(w)) return "url";
  if (isEmail(w)) return "email";
  if (isUuid(w)) return "uuid";
  if (isIban(w)) return "iban";
  if (isToken(w)) return "token";
  if (isCode(w)) return "code";
}
function luhn(d) {
  let sum = 0;
  for (let i = 0; i < d.length; i++) {
    let x = +d[d.length - 1 - i];
    if (i % 2 === 1) {
      x *= 2;
      if (x > 9) x -= 9;
    }
    sum += x;
  }
  return sum % 10 === 0;
}
var NUM_SEPS = " .-/,:\xA0\u202F";
var groupedByThousands = (run) => {
  const g = run.split(/[ .\-/,:\u00a0\u202f]/);
  return g.length > 1 && g[0].length >= 1 && g[0].length <= 3 && g.slice(1).every((x) => x.length === 3);
};
function numberKind(run) {
  const digits = run.replace(/[^0-9]/g, "");
  const n = digits.length;
  if (n >= 13 && n <= 19 && luhn(digits)) return "card";
  if (n >= 9 && !groupedByThousands(run.replace(/^\++/, ""))) return run.startsWith("+") || run.startsWith("0") ? "phone" : "long_number";
  return "number";
}
var STREET = [
  "rue",
  "avenue",
  "av.",
  "boulevard",
  "bd",
  "chemin",
  "all\xE9e",
  "impasse",
  "quai",
  "street",
  "road",
  "lane",
  "drive",
  "calle",
  "avenida",
  "via",
  "strasse",
  "stra\xDFe",
  "straat",
  "rua"
];
var splitSep = (t) => t.split(/[ \t\n\r\f  ]/).filter((w) => w !== "");
function looksLikeAddress(text) {
  const words = splitSep(text).map((w) => w.replace(/,+$/, "").toLowerCase());
  const house = (w) => {
    const num = w.replace(/[A-Za-z]+$/, "");
    return num !== "" && num.length <= 4 && w.length - num.length <= 3 && /^[0-9]+$/.test(num);
  };
  return words.some((w, j) => STREET.includes(w) && words.slice(Math.max(0, j - 3), j).some(house));
}
var LEAD_PUNCT = `([{\xAB"'\u201C\u2018`;
var TRAIL_PUNCT = `.,;:!?)]}\xBB"'\u201D\u2019`;
var prevChar = (s, i) => i <= 0 ? "" : String.fromCodePoint(s.codePointAt(i >= 2 && /[\udc00-\udfff]/.test(s[i - 1]) && /[\ud800-\udbff]/.test(s[i - 2]) ? i - 2 : i - 1));
var nextChar = (s, i) => i >= s.length ? "" : String.fromCodePoint(s.codePointAt(i));
function mask(text, values, allow) {
  const hits = [];
  let pos = 0;
  while (pos < text.length) {
    let start = pos;
    while (start < text.length && isSep(text[start])) start++;
    if (start >= text.length) break;
    let end = start;
    while (end < text.length && !isSep(text[end])) end++;
    let cs = start;
    while (cs < end && LEAD_PUNCT.includes(text[cs])) cs++;
    let ce = end;
    while (ce > cs && TRAIL_PUNCT.includes(text[ce - 1])) ce--;
    if (text.slice(cs, ce).includes("(") && text[ce] === ")" && ce < end) ce++;
    if (cs < ce) {
      const k = wordKind(text.slice(cs, ce));
      if (k) hits.push([cs, ce, k]);
    }
    pos = end;
  }
  const runs = [];
  let p = 0;
  while (p < text.length) {
    if (!isDigit(text[p])) {
      p++;
      continue;
    }
    let start = p;
    if (start > 0 && text[start - 1] === "+" && (start === 1 || isSep(text[start - 2]))) start--;
    let end = p + 1;
    for (; ; ) {
      if (isDigit(text[end])) {
        end++;
        continue;
      }
      if (end < text.length && NUM_SEPS.includes(text[end]) && isDigit(text[end + 1])) {
        end += 2;
        continue;
      }
      break;
    }
    const before = prevChar(text, start);
    const after = nextChar(text, end);
    const beforeOk = !(before && (ALNUM.test(before) || before === "_"));
    const afterOk = !(after && (ALPHA.test(after) || after === "_"));
    if (beforeOk && afterOk && !hits.some((h) => h[0] < end && start < h[1])) runs.push([start, end, numberKind(text.slice(start, end))]);
    p = end;
  }
  hits.push(...runs);
  hits.sort((a, b2) => a[0] - b2[0]);
  let out = "";
  let last = 0;
  for (const [s, e, k0] of hits) {
    const v = text.slice(s, e);
    const kind = isPii(k0) && allow.some((a) => eqIgnoreAscii(a, v)) ? "public" : k0;
    out += text.slice(last, s) + `{${values.length}}`;
    values.push({ kind, value: v });
    last = e;
  }
  return out + text.slice(last);
}
var get = (attrs, n) => attrs.find(([k]) => k === n)?.[1];
var classes = (v) => (v ?? "").split(/[ \t\n\r\f]+/).filter(Boolean);
function parseSelector(s0) {
  const s = s0.trim();
  if (!s || /[ >+~,]/.test(s)) return;
  const sel = { classes: [], attrs: [] };
  const identEnd = (r) => {
    const i = r.search(/[.#[]/);
    return i < 0 ? r.length : i;
  };
  let rest = s;
  const n = identEnd(rest);
  if (n > 0) {
    const t = rest.slice(0, n);
    if (t !== "*") sel.tag = asciiLower(t);
    rest = rest.slice(n);
  }
  while (rest) {
    const c = rest[0];
    rest = rest.slice(1);
    if (c === "." || c === "#") {
      const n2 = identEnd(rest);
      if (!n2) return;
      if (c === ".") sel.classes.push(rest.slice(0, n2));
      else sel.id = rest.slice(0, n2);
      rest = rest.slice(n2);
    } else if (c === "[") {
      const close = rest.indexOf("]");
      if (close < 0) return;
      const inner = rest.slice(0, close);
      rest = rest.slice(close + 1);
      const eq = inner.indexOf("=");
      if (eq >= 0) sel.attrs.push([asciiLower(inner.slice(0, eq).trim()), inner.slice(eq + 1).trim().replace(/^["']+|["']+$/g, "")]);
      else sel.attrs.push([asciiLower(inner.trim()), void 0]);
    } else return;
  }
  return sel;
}
var selMatches = (s, tag, attrs) => (s.tag === void 0 || s.tag === tag) && (s.id === void 0 || get(attrs, "id") === s.id) && s.classes.every((c) => classes(get(attrs, "class")).includes(c)) && s.attrs.every(([k, v]) => {
  const a = get(attrs, k);
  return a !== void 0 && (v === void 0 || a === v);
});
var CODE_TAGS = ["script", "style", "pre", "code", "kbd", "samp", "var", "svg", "math", "xmp", "plaintext"];
function skipOf(tag, attrs, sels) {
  const ce = get(attrs, "contenteditable");
  if (eqIgnoreAscii(get(attrs, "translate") ?? "", "no") || classes(get(attrs, "class")).includes("notranslate") || get(attrs, "data-glot-skip") !== void 0 || get(attrs, "data-glot-user") !== void 0 || ce !== void 0 && !eqIgnoreAscii(ce, "false") || sels.some((s) => selMatches(s, tag, attrs))) return "opaque";
  if (CODE_TAGS.includes(tag)) return "code";
  if (tag === "textarea" || tag === "select") return "form";
}
var RAW_TEXT = ["script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes"];
var asciiWs = (c) => c !== void 0 && isWs(c);
function tokenize(s) {
  const n = s.length;
  const lower = asciiLower(s);
  const toks = [];
  let textStart = 0;
  let i = 0;
  const find = (from, pat) => {
    const j = s.indexOf(pat, from);
    return j < 0 ? void 0 : j;
  };
  const nameEnd = (from) => {
    let j = from;
    while (j < n && !asciiWs(s[j]) && s[j] !== "/" && s[j] !== ">") j++;
    return j;
  };
  const flush = () => {
    if (textStart < i) toks.push({ t: "text", range: [textStart, i] });
  };
  while (i < n) {
    if (s[i] !== "<" || i + 1 >= n) {
      i++;
      continue;
    }
    const c = s[i + 1];
    if (s.startsWith("<!--", i)) {
      flush();
      const j = find(i + 4, "-->");
      const end = j === void 0 ? n : j + 3;
      toks.push({ t: "comment", range: [i, end] });
      i = textStart = end;
    } else if (c === "!" || c === "?") {
      flush();
      const j = find(i, ">");
      const end = j === void 0 ? n : j + 1;
      toks.push({ t: "comment", range: [i, end] });
      i = textStart = end;
    } else if (c === "/" && i + 2 < n && isAsciiAlpha(s[i + 2])) {
      flush();
      const ne = nameEnd(i + 2);
      const j = find(ne, ">");
      const end = j === void 0 ? n : j + 1;
      toks.push({ t: "end", name: lower.slice(i + 2, ne), range: [i, end] });
      i = textStart = end;
    } else if (isAsciiAlpha(c)) {
      flush();
      const ne = nameEnd(i + 1);
      const name = lower.slice(i + 1, ne);
      let j = ne;
      const attrs = [];
      let selfClosing = false;
      for (; ; ) {
        while (j < n && (asciiWs(s[j]) || s[j] === "/")) {
          selfClosing = s[j] === "/";
          j++;
        }
        if (j >= n || s[j] === ">") break;
        selfClosing = false;
        const ns = j;
        j++;
        while (j < n && !asciiWs(s[j]) && s[j] !== "=" && s[j] !== ">" && s[j] !== "/") j++;
        const aname = lower.slice(ns, j);
        let k = j;
        while (k < n && asciiWs(s[k])) k++;
        let value = "";
        if (k < n && s[k] === "=") {
          k++;
          while (k < n && asciiWs(s[k])) k++;
          if (k < n && (s[k] === '"' || s[k] === "'")) {
            const q = s[k];
            const vs = k + 1;
            let ve = s.indexOf(q, vs);
            if (ve < 0) ve = n;
            value = s.slice(vs, ve);
            j = Math.min(ve + 1, n);
          } else {
            const vs = k;
            while (k < n && !asciiWs(s[k]) && s[k] !== ">") k++;
            value = s.slice(vs, k);
            j = k;
          }
        }
        attrs.push({ name: aname, value: decodeEntities(value), range: [ns, j] });
      }
      const end = Math.min(j + 1, n);
      toks.push({ t: "start", name, attrs, selfClosing, range: [i, end] });
      i = textStart = end;
      if (RAW_TEXT.includes(name) && !selfClosing) {
        const pat = `</${name}`;
        let from = i;
        let close;
        for (; ; ) {
          const x = lower.indexOf(pat, from);
          if (x < 0) {
            close = n;
            break;
          }
          const after = x + pat.length;
          if (after >= n || asciiWs(s[after]) || s[after] === "/" || s[after] === ">") {
            close = x;
            break;
          }
          from = after;
        }
        if (i < close) toks.push({ t: "text", range: [i, close] });
        i = textStart = close;
      }
    } else i++;
  }
  flush();
  return toks;
}
var VOID = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
var INLINE = [
  "a",
  "abbr",
  "b",
  "bdi",
  "bdo",
  "big",
  "br",
  "button",
  "cite",
  "code",
  "data",
  "del",
  "dfn",
  "em",
  "font",
  "i",
  "img",
  "input",
  "ins",
  "kbd",
  "label",
  "link",
  "mark",
  "meta",
  "meter",
  "output",
  "picture",
  "progress",
  "q",
  "ruby",
  "rp",
  "rt",
  "s",
  "samp",
  "select",
  "small",
  "span",
  "strike",
  "strong",
  "sub",
  "sup",
  "textarea",
  "time",
  "tt",
  "u",
  "var",
  "wbr",
  "svg",
  "math",
  "video",
  "audio",
  "canvas",
  "iframe",
  "object",
  "embed"
];
var MEDIA = ["img", "picture", "video", "audio", "canvas", "iframe", "object", "embed"];
var ROLE_INLINE = ["a", "button", "label"];
var META_NAMES = ["description", "og:title", "og:description", "twitter:title", "twitter:description"];
var P_CLOSERS = [
  "address",
  "article",
  "aside",
  "blockquote",
  "details",
  "dialog",
  "div",
  "dl",
  "dd",
  "dt",
  "fieldset",
  "figcaption",
  "figure",
  "footer",
  "form",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "header",
  "hgroup",
  "hr",
  "li",
  "main",
  "menu",
  "nav",
  "ol",
  "p",
  "pre",
  "section",
  "table",
  "ul"
];
var NORMAL = 0;
var OPAQUE = 1;
var SKIP = 2;
var trimmable = (it) => it.t === "text" ? allWs(it.text) : it.t === "void" || it.t === "comment";
var roleOf = (tag, attrs) => {
  const r = asciiLower((get(attrs, "role") ?? "").trim());
  return r || tag;
};
var keyOf = (attrs) => get(attrs, "data-glot-key") ?? get(attrs, "data-glot");
function impliedClose(open, n) {
  switch (open) {
    case "p":
      return P_CLOSERS.includes(n);
    case "li":
      return n === "li";
    case "dt":
    case "dd":
      return n === "dt" || n === "dd";
    case "td":
    case "th":
      return ["td", "th", "tr", "tbody", "thead", "tfoot"].includes(n);
    case "tr":
      return ["tr", "tbody", "thead", "tfoot"].includes(n);
    case "thead":
    case "tbody":
    case "tfoot":
      return ["tbody", "thead", "tfoot"].includes(n);
  }
  return false;
}
function parseTemplate(t, ntags, nvalues) {
  const out = [];
  const re = /<(\/?)(\d+)(\/?)>|\{(\d+)\}/g;
  let last = 0;
  for (let m; m = re.exec(t); ) {
    let part;
    if (m[4] !== void 0) {
      const n = +m[4];
      if (n < nvalues) part = { p: "val", n };
    } else if (!(m[1] && m[3])) {
      const n = +m[2];
      if (n < ntags) part = { p: m[1] ? "close" : m[3] ? "void" : "open", n };
    }
    if (!part) {
      re.lastIndex = m.index + 1;
      continue;
    }
    if (last < m.index) out.push({ p: "text", s: t.slice(last, m.index) });
    out.push(part);
    last = re.lastIndex;
  }
  if (last < t.length) out.push({ p: "text", s: t.slice(last) });
  return out;
}
var stripPlaceholders = (t) => parseTemplate(t, Infinity, Infinity).filter((p) => p.p === "text").map((p) => p.s).join(" ");
var Segmenter = class {
  stack = [];
  run = [];
  nextId = 0;
  out = [];
  opts;
  sels;
  constructor(opts, sels) {
    this.opts = opts;
    this.sels = sels;
  }
  level() {
    return this.stack.length ? this.stack[this.stack.length - 1].level : NORMAL;
  }
  start(name, attrs, selfClosing, range) {
    const kv = attrs.map((a) => [a.name, a.value]);
    if (this.level() === NORMAL) {
      for (; ; ) {
        let i = -1;
        for (let j = this.stack.length - 1; j >= 0; j--) if (this.stack[j].kind.k === "block") {
          i = j;
          break;
        }
        if (i < 0) break;
        const onlyInline = this.stack.slice(i + 1).every((f) => f.kind.k === "inline");
        if (onlyInline && impliedClose(this.stack[i].name, name)) this.closeTo(i, [range[0], range[0]]);
        else break;
      }
    }
    const cur = this.level();
    const skip = cur === SKIP ? void 0 : skipOf(name, kv, this.sels);
    if (cur !== SKIP && skip !== "code" && skip !== "opaque") this.attrSegments(name, attrs, kv);
    const isVoid = VOID.includes(name) || selfClosing && (cur === SKIP || name === "svg" || name === "math");
    const own = skip ? SKIP : MEDIA.includes(name) ? OPAQUE : NORMAL;
    const level = Math.max(cur, own);
    const inline = INLINE.includes(name);
    let kind;
    if (cur !== NORMAL) kind = { k: "muted" };
    else if (level === NORMAL && inline) {
      if (isVoid) {
        this.run.push({ t: "void", name, range });
        return;
      }
      const id = this.nextId++;
      this.run.push({ t: "open", id, name, role: ROLE_INLINE.includes(name) || get(kv, "role") !== void 0 ? roleOf(name, kv) : void 0, key: keyOf(kv), range });
      kind = { k: "inline", id };
    } else if (inline) {
      this.run.push({ t: "void", name, range });
      kind = { k: "voidEl", item: this.run.length - 1 };
    } else {
      for (let j = this.stack.length - 1; j >= 0; j--) {
        const k = this.stack[j].kind;
        if (k.k === "block") {
          k.hasChildBlock = true;
          break;
        }
      }
      this.flush();
      kind = level === NORMAL ? { k: "block", role: roleOf(name, kv), key: keyOf(kv), hasChildBlock: false } : { k: "muted" };
    }
    if (!isVoid) this.stack.push({ name, level, kind });
  }
  end(name, range) {
    for (let i = this.stack.length - 1; i >= 0; i--) if (this.stack[i].name === name) return this.closeTo(i, range);
  }
  closeTo(i, range) {
    while (this.stack.length > i) {
      const r = this.stack.length === i + 1 ? range : [range[0], range[0]];
      const k = this.stack[this.stack.length - 1].kind;
      if (k.k === "inline") this.run.push({ t: "close", id: k.id, range: r });
      else if (k.k === "voidEl") {
        const it = this.run[k.item];
        if (it?.t === "void") it.range = [it.range[0], r[1]];
      } else if (k.k === "block") this.flush();
      this.stack.pop();
    }
  }
  attrSegments(tag, attrs, kv) {
    for (const a of attrs) {
      let role;
      if (["title", "alt", "aria-label", "aria-description", "placeholder"].includes(a.name)) role = `${tag}:${a.name}`;
      else if (a.name === "value" && tag === "input" && ["button", "submit", "reset"].includes(asciiLower(get(kv, "type") ?? "-"))) role = "input:value";
      else if (a.name === "content" && tag === "meta") {
        const n = get(kv, "name") ?? get(kv, "property");
        if (n === void 0 || !META_NAMES.includes(asciiLower(n))) continue;
        role = `meta:${asciiLower(n)}`;
      } else continue;
      const values = [];
      const text = collapseWs(mask(a.value, values, this.opts.piiAllow));
      const seg = this.make(text, values, a.value, role, void 0);
      if (seg) {
        seg.span = a.range;
        seg.attr = a.name;
        this.out.push(seg);
      }
    }
  }
  make(text, values, raw, role, key) {
    let plain = stripPlaceholders(text);
    for (const t of this.opts.dnt) if (t) plain = plain.split(t).join(" ");
    if (!hasLetter(plain)) return;
    const discoverable = !values.some((v) => isPii(v.kind)) && !looksLikeAddress(raw);
    const seg = { hash: "", text, role, discoverable, values, tags: [], span: [0, 0] };
    if (key !== void 0) seg.key = key;
    return seg;
  }
  carrier() {
    for (let j = this.stack.length - 1; j >= 0; j--) {
      const k = this.stack[j].kind;
      if (k.k === "block") return [k.role, k.hasChildBlock ? void 0 : k.key];
    }
    return ["body", void 0];
  }
  flush() {
    if (!this.run.length) return;
    const items = [];
    for (const it of this.run) {
      if (it.t === "close") {
        const last = items[items.length - 1];
        const at = items.length - (last?.t === "text" && allWs(last.text) ? 1 : 0);
        const o = items[at - 1];
        if (o?.t === "open" && o.id === it.id) {
          items.length = at - 1;
          items.push({ t: "void", name: o.name, range: [o.range[0], it.range[1]] });
          continue;
        }
      }
      items.push(it);
    }
    this.run = [];
    const [role, key] = this.carrier();
    const opened = /* @__PURE__ */ new Set(), matched = /* @__PURE__ */ new Set();
    for (const it of items) {
      if (it.t === "open") opened.add(it.id);
      else if (it.t === "close" && opened.has(it.id)) matched.add(it.id);
    }
    let start = 0;
    items.forEach((it, i) => {
      if ((it.t === "open" || it.t === "close") && !matched.has(it.id)) {
        this.piece(items.slice(start, i), role, key);
        start = i + 1;
      }
    });
    this.piece(items.slice(start), role, key);
  }
  piece(s, role, key) {
    for (; ; ) {
      while (s.length && trimmable(s[0])) s = s.slice(1);
      while (s.length && trimmable(s[s.length - 1])) s = s.slice(0, -1);
      const f = s[0], l = s[s.length - 1];
      if (s.length >= 2 && f.t === "open" && l.t === "close" && f.id === l.id) {
        if (f.role !== void 0) role = f.role;
        if (f.key !== void 0) key = f.key;
        s = s.slice(1, -1);
      } else break;
    }
    if (!s.length) return;
    let depth = 0, loose = false, spaced = false, allRoles = true, gstart = 0;
    const groups = [];
    s.forEach((it, i) => {
      if (it.t === "open") {
        if (depth === 0) {
          gstart = i;
          allRoles &&= it.role !== void 0;
        }
        depth++;
      } else if (it.t === "close") {
        depth = Math.max(0, depth - 1);
        if (depth === 0) groups.push([gstart, i + 1]);
      } else if (it.t === "text" && depth === 0) {
        const ws = allWs(it.text);
        loose ||= !ws;
        spaced ||= ws;
      }
    });
    if (!loose && (allRoles || !spaced)) {
      for (const [a, b2] of groups) this.piece(s.slice(a, b2), role, void 0);
      return;
    }
    this.emit(s, role, key);
  }
  emit(s, role, key) {
    let text = "", raw = "";
    const values = [];
    const tags = [];
    const ids = /* @__PURE__ */ new Map();
    for (const it of s) {
      if (it.t === "text") {
        raw += it.text;
        text += mask(it.text, values, this.opts.piiAllow);
      } else if (it.t === "open") {
        ids.set(it.id, tags.length);
        text += `<${tags.length}>`;
        tags.push({ pair: true, name: it.name, open: it.range, close: [0, 0] });
      } else if (it.t === "close") {
        const k = ids.get(it.id);
        tags[k].close = it.range;
        text += `</${k}>`;
      } else if (it.t === "void") {
        text += `<${tags.length}/>`;
        tags.push({ pair: false, name: it.name, range: it.range });
      }
      raw += " ";
    }
    const seg = this.make(collapseWs(text), values, raw, role, key);
    if (seg) {
      seg.tags = tags;
      seg.span = [s[0].range[0], s[s.length - 1].range[1]];
      this.out.push(seg);
    }
  }
};
var fullOpts = (o) => ({ sourceLocale: o.sourceLocale ?? "en", dnt: o.dnt ?? [], piiAllow: o.piiAllow ?? [] });
function segment(html, opts = {}) {
  const sels = (opts.skip ?? []).map(parseSelector).filter((s) => !!s);
  const sg = new Segmenter(fullOpts(opts), sels);
  for (const t of tokenize(html)) {
    if (t.t === "start") sg.start(t.name, t.attrs, t.selfClosing, t.range);
    else if (t.t === "end") sg.end(t.name, t.range);
    else if (sg.level() === NORMAL) {
      if (t.t === "text") sg.run.push({ t: "text", range: t.range, text: decodeEntities(html.slice(t.range[0], t.range[1])) });
      else sg.run.push({ t: "comment", range: t.range });
    }
  }
  sg.closeTo(0, [html.length, html.length]);
  sg.flush();
  return sg.out.sort((a, b2) => a.span[0] - b2.span[0]);
}
var segmentKey = (sourceLocale, text) => hash16(`${sourceLocale}\0${text}`);
function segmentHashed(html, opts = {}) {
  const segs = segment(html, opts);
  for (const s of segs) s.hash = segmentKey(opts.sourceLocale ?? "en", s.text);
  return segs;
}
var K = new Uint32Array(64);
var isPrime = (n) => {
  for (let d = 2; d * d <= n; d++) if (n % d === 0) return false;
  return true;
};
for (let n = 2, i = 0; i < 64; n++) if (isPrime(n)) K[i++] = Math.cbrt(n) % 1 * 2 ** 32;
var H0 = [1779033703, 3144134277, 1013904242, 2773480762, 1359893119, 2600822924, 528734635, 1541459225];
function hash16(s) {
  const m = new TextEncoder().encode(s), l = m.length, n = l + 9 + 63 >> 6 << 6, b2 = new Uint8Array(n);
  b2.set(m);
  b2[l] = 128;
  new DataView(b2.buffer).setUint32(n - 4, l * 8);
  const h = H0.slice(), w = new Uint32Array(64), r = (x, k) => x >>> k | x << 32 - k;
  for (let o = 0; o < n; o += 64) {
    for (let i = 0; i < 16; i++) w[i] = b2[o + 4 * i] << 24 | b2[o + 4 * i + 1] << 16 | b2[o + 4 * i + 2] << 8 | b2[o + 4 * i + 3];
    for (let i = 16; i < 64; i++) w[i] = w[i - 16] + (r(w[i - 15], 7) ^ r(w[i - 15], 18) ^ w[i - 15] >>> 3) + w[i - 7] + (r(w[i - 2], 17) ^ r(w[i - 2], 19) ^ w[i - 2] >>> 10);
    let [a, bb, c, d, e, f, g, hh] = h;
    for (let i = 0; i < 64; i++) {
      const t1 = hh + (r(e, 6) ^ r(e, 11) ^ r(e, 25)) + (e & f ^ ~e & g) + K[i] + w[i] | 0;
      const t2 = (r(a, 2) ^ r(a, 13) ^ r(a, 22)) + (a & bb ^ a & c ^ bb & c) | 0;
      hh = g;
      g = f;
      f = e;
      e = d + t1 | 0;
      d = c;
      c = bb;
      bb = a;
      a = t1 + t2 | 0;
    }
    [a, bb, c, d, e, f, g, hh].forEach((x, i) => h[i] = h[i] + x | 0);
  }
  return h.slice(0, 4).map((x) => (x >>> 0).toString(16).padStart(8, "0")).join("");
}
function template(path, locale) {
  const p = path.split("/").filter(Boolean);
  if (p[0]?.toLowerCase() === locale.toLowerCase()) p.shift();
  return "/" + p.map((s, i) => {
    const digits = s.replace(/\D/g, "").length;
    if (digits === s.length || s.length >= 8 && /^[0-9a-f-]+$/i.test(s) && digits > 0 || s.length >= 24) return ":id";
    return p.length >= 2 && i === p.length - 1 && /[-_]/.test(s) ? "*" : s.toLowerCase();
  }).join("/");
}
function check(seg, tr2) {
  const parts = parseTemplate(tr2, Infinity, Infinity);
  const seenTag = new Array(seg.tags.length).fill(0), seenVal = new Array(seg.values.length).fill(0);
  const stack = [];
  for (const p of parts) {
    if (p.p === "text") continue;
    if (p.p === "val") {
      if (p.n >= seg.values.length) return;
      seenVal[p.n]++;
      continue;
    }
    if (p.n >= seg.tags.length) return;
    if (p.p === "open") {
      if (!seg.tags[p.n].pair) return;
      seenTag[p.n]++;
      stack.push(p.n);
    } else if (p.p === "close") {
      if (stack.pop() !== p.n) return;
    } else {
      if (seg.tags[p.n].pair) return;
      seenTag[p.n]++;
    }
  }
  return !stack.length && seenTag.every((n) => n === 1) && seenVal.every((n) => n === 1) && (!seg.attr || !seg.tags.length) ? parts : void 0;
}
function splice(src, [lo, hi], edits) {
  let out = "", at = lo;
  for (const [[s, e], v] of edits) if (s >= at && e <= hi) {
    out += src.slice(at, s) + v;
    at = e;
  }
  return out + src.slice(at, hi);
}
function translateHtml(html, opts, lookup2) {
  const segs = segmentHashed(html, opts);
  const attrEdits = [];
  const textTr = [];
  for (const seg of segs) {
    const t = lookup2(seg);
    const parts = t === void 0 ? void 0 : check(seg, t);
    if (!parts) continue;
    if (seg.attr) {
      const v = parts.map((p) => p.p === "val" ? seg.values[p.n].value : p.p === "text" ? p.s : "").join("");
      attrEdits.push([seg.span, `${seg.attr}="${escapeAttr(v)}"`]);
    } else textTr.push([seg, parts]);
  }
  attrEdits.sort((a, b2) => a[0][0] - b2[0][0]);
  const edits = textTr.map(([seg, parts]) => {
    let out = "";
    for (const p of parts) {
      if (p.p === "text") out += escapeText(p.s);
      else if (p.p === "val") out += escapeText(seg.values[p.n].value);
      else {
        const tag = seg.tags[p.n];
        out += splice(html, tag.pair ? p.p === "close" ? tag.close : tag.open : tag.range, attrEdits);
      }
    }
    return [seg.span, out];
  });
  const covered = edits.map((e) => e[0]);
  for (const e of attrEdits) if (!covered.some(([s, t]) => s <= e[0][0] && e[0][1] <= t)) edits.push(e);
  edits.sort((a, b2) => a[0][0] - b2[0][0]);
  return splice(html, [0, html.length], edits);
}

// src/hydration.ts
var ID_KEYS = /^(id|_id|key|slug|href|url|src|srcset|as|type|kind|variant|status|state|plan|tier|mode|lang|locale|code|name|className|class|style|role|rel|target|method|action|icon|color|size|format|currency|path|route|pathname|query|hash)$/i;
var fold = (s) => s.replace(/\s+/g, " ").trim();
var esc = (json) => json.replace(/[<>&\u2028\u2029]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`);
function tr(s, lookup2) {
  if (!/\p{L}/u.test(s) || s.startsWith("$")) return void 0;
  const t = lookup2(fold(s));
  return t === void 0 || t === s ? void 0 : t;
}
function walk(v, lookup2, n, key = "") {
  if (typeof v === "string") {
    if (ID_KEYS.test(key)) return v;
    const t = tr(v, lookup2);
    if (t !== void 0) n.c++;
    return t ?? v;
  }
  if (Array.isArray(v)) return v.map((x) => walk(x, lookup2, n, key));
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, walk(x, lookup2, n, k)]));
  return v;
}
function nextData(html, lookup2, n) {
  return html.replace(/(<script\b[^>]*\bid="__NEXT_DATA__"[^>]*>)([\s\S]*?)(<\/script>)/, (all, open, body, close) => {
    try {
      return open + esc(JSON.stringify(walk(JSON.parse(body), lookup2, n))) + close;
    } catch {
      return all;
    }
  });
}
function nuxtData(html, lookup2, n) {
  return html.replace(/(<script\b[^>]*\bid="__NUXT_DATA__"[^>]*>)([\s\S]*?)(<\/script>)/, (all, open, body, close) => {
    try {
      const a = JSON.parse(body);
      if (!Array.isArray(a)) return all;
      const skip = /* @__PURE__ */ new Set();
      for (const x of a) {
        if (x && typeof x === "object" && !Array.isArray(x)) {
          for (const [k, i] of Object.entries(x)) if (ID_KEYS.test(k) && typeof i === "number") skip.add(i);
        }
        if (Array.isArray(x) && typeof x[0] === "string") {
          for (const i of x.slice(1)) if (typeof i === "number") skip.add(i);
        }
      }
      const out = a.map((x, i) => {
        if (typeof x !== "string" || skip.has(i)) return x;
        const t = tr(x, lookup2);
        if (t !== void 0) n.c++;
        return t ?? x;
      });
      return open + esc(JSON.stringify(out)) + close;
    } catch {
      return all;
    }
  });
}
var PUSH = /<script\b[^>]*>\s*self\.__next_f\.push\((\[1,"(?:[^"\\]|\\.)*"\])\)\s*<\/script>/g;
var utf8Len = (s) => new TextEncoder().encode(s).length;
function flightEdits(stream, lookup2) {
  const edits = [];
  let i = 0;
  while (i < stream.length) {
    if (stream[i] === "\n") {
      i++;
      continue;
    }
    const m = /^([0-9a-f]*):/.exec(stream.slice(i, i + 24));
    if (!m) break;
    let j = i + m[0].length;
    if (stream[j] === "T") {
      const comma = stream.indexOf(",", j);
      if (comma < 0) break;
      let bytes = parseInt(stream.slice(j + 1, comma), 16);
      j = comma + 1;
      while (bytes > 0 && j < stream.length) {
        const cp = stream.codePointAt(j);
        bytes -= utf8Len(String.fromCodePoint(cp));
        j += cp > 65535 ? 2 : 1;
      }
      i = j;
      continue;
    }
    const end = stream.indexOf("\n", j);
    const row = end < 0 ? stream.slice(j) : stream.slice(j, end);
    if (row.startsWith("[") || row.startsWith("{")) {
      const tok = /"((?:[^"\\]|\\.)*)"(\s*:)?/g;
      let key = "";
      for (let t; t = tok.exec(row); ) {
        if (t[2]) {
          key = JSON.parse(`"${t[1]}"`);
          continue;
        }
        const s = JSON.parse(`"${t[1]}"`);
        const k = key;
        key = "";
        if (ID_KEYS.test(k)) continue;
        const v = tr(s, lookup2);
        if (v !== void 0) edits.push([j + t.index, j + t.index + t[0].length, JSON.stringify(v)]);
      }
    }
    if (end < 0) break;
    i = end + 1;
  }
  return edits;
}
function flight(html, lookup2, n) {
  const pushes = [...html.matchAll(PUSH)];
  if (!pushes.length) return html;
  const chunks = pushes.map((p) => JSON.parse(p[1])[1]);
  const starts = [];
  let off = 0;
  for (const c of chunks) starts.push(off), off += c.length;
  const edits = flightEdits(chunks.join(""), lookup2);
  const out = chunks.slice();
  for (const [a, b2, v] of edits.reverse()) {
    let k = starts.length - 1;
    while (starts[k] > a) k--;
    if (b2 > starts[k] + chunks[k].length) continue;
    out[k] = out[k].slice(0, a - starts[k]) + v + out[k].slice(b2 - starts[k]);
    n.c++;
  }
  let idx = 0;
  return html.replace(PUSH, (all) => {
    const k = idx++;
    return out[k] === chunks[k] ? all : all.replace(pushes[k][1], () => esc(JSON.stringify([1, out[k]])));
  });
}
function translatePayloads(html, lookup2) {
  const n = { c: 0 };
  let out = html;
  if (out.includes("__NEXT_DATA__")) out = nextData(out, lookup2, n);
  if (out.includes("__NUXT_DATA__")) out = nuxtData(out, lookup2, n);
  if (out.includes("self.__next_f.push")) out = flight(out, lookup2, n);
  return { html: out, replaced: n.c };
}
var attr = (s) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
var patcherTag = (scriptUrl, tmUrl, sourceLocale, missing) => `<script async src="${attr(scriptUrl)}" data-tm="${attr(tmUrl)}" data-source="${attr(sourceLocale)}"${missing ? ` data-missing="${attr(missing)}"` : ""}></script>`;
function injectHead(html, tag) {
  const m = /<meta name="glot-tm"[^>]*>/i.exec(html) ?? /<head(?:\s[^>]*)?>/i.exec(html);
  return m ? html.slice(0, m.index + m[0].length) + tag + html.slice(m.index + m[0].length) : html;
}

// src/translate.ts
var translate = (html, tm, c) => {
  const tpl = template(c.path, tm.locale);
  let total = 0, translated = 0;
  const missing = [];
  const seen = [];
  const payload = /* @__PURE__ */ new Map();
  let out = translateHtml(html, { sourceLocale: c.source, skip: c.skip }, (s) => {
    total++;
    seen.push({ hash: s.hash, role: s.role });
    const v = lookup(tm, s.hash, `${s.role}|${tpl}`);
    if (v === void 0) {
      if (s.discoverable) missing.push({ hash: s.hash, text: s.text, role: s.role });
      return void 0;
    }
    translated++;
    if (!s.tags.length && !s.values.length && !s.attr) payload.set(s.text, v);
    return v;
  });
  out = translatePayloads(out, (src) => payload.get(src)).html;
  out = seo(out, {
    source: c.source,
    locale: tm.locale,
    bases: subdirBases(c.url.origin, c.source, c.locales),
    path: c.path + c.url.search,
    tr: (s) => lookup(tm, hash16(`${c.source}\0${collapseWs(s)}`), `text|${tpl}`)
  });
  out = injectHead(out, `<meta name="glot-tm" content="${esc2(`${tm.locale}@${tm.v}`)}">`);
  out = injectHead(out, patcherTag("/__glot/patcher.js", `/__glot/tm.json?locale=${encodeURIComponent(tm.locale)}`, c.source, c.discover ? "/__glot/missing" : void 0));
  return { html: out, total, translated, missing, seen };
};
var subdirBases = (origin, source, locales) => [source, ...locales.filter((l) => l !== source)].map((l) => [l, l === source ? origin : `${origin}/${l}`]);
var RTL = /^(ar|he|iw|fa|ur|ps|sd|ug|yi|dv|ckb)$/;
var dir = (l) => RTL.test(l.split(/[-_]/)[0]) ? "rtl" : "ltr";
var esc2 = (s) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
var urlPath = (b2) => {
  const i = b2.indexOf("://");
  if (i < 0) return void 0;
  const r = b2.slice(i + 3), j = r.indexOf("/");
  return (j < 0 ? "" : r.slice(j)).replace(/\/+$/, "");
};
var hostOf = (b2) => {
  const i = b2.indexOf("://");
  return (i < 0 ? b2 : b2.slice(i + 3)).split("/")[0];
};
var originOf = (b2) => b2.slice(0, b2.length - (urlPath(b2) ?? "").length);
function stripDir(href, d) {
  const p = d.replace(/\/+$/, "");
  if (!href.startsWith(p)) return void 0;
  const rest = href.slice(p.length);
  return rest === "" || "/?#".includes(rest[0]) ? rest : void 0;
}
function isAsset(path) {
  const p = path.split(/[?#]/)[0];
  if (p.startsWith("/_next/") || p.startsWith("/api/") || p.startsWith("/cdn-cgi/")) return true;
  const last = p.slice(p.lastIndexOf("/") + 1), dot = last.lastIndexOf(".");
  return dot >= 0 && !["html", "htm", "php", "aspx"].includes(last.slice(dot + 1).toLowerCase());
}
var base = (s, l) => s.bases.find(([x]) => x === l)?.[1].replace(/\/+$/, "");
var urlFor = (s, l, path) => {
  const b2 = base(s, l);
  return b2 === void 0 ? void 0 : b2 + (path || "/");
};
function sitePath(s, href0) {
  const href = href0.trim();
  let path;
  if (href.startsWith("/") && !href.startsWith("//")) {
    path = href;
    for (const [, b2] of s.bases) {
      const pre = urlPath(b2);
      const rest = pre ? stripDir(href, pre) : void 0;
      if (rest !== void 0) path = rest;
    }
  } else {
    for (const [, b2] of [...s.bases].sort((x, y) => y[1].length - x[1].length)) if ((path = stripDir(href, b2)) !== void 0) break;
    if (path === void 0) return void 0;
  }
  path ||= "/";
  return isAsset(path) ? void 0 : path;
}
function localizeHref(s, href) {
  const path = sitePath(s, href), b2 = base(s, s.locale);
  if (path === void 0 || b2 === void 0) return void 0;
  if (!href.trim().startsWith("/")) return b2 + path;
  const src = base(s, s.source) ?? "";
  if (hostOf(b2) !== hostOf(src)) return path;
  return (urlPath(b2) ?? "") + (stripDir(path, urlPath(src) ?? "") ?? path);
}
var LD_TEXT = [
  "name",
  "alternateName",
  "description",
  "headline",
  "alternativeHeadline",
  "abstract",
  "text",
  "caption",
  "slogan",
  "disambiguatingDescription",
  "articleBody",
  "reviewBody",
  "keywords"
];
var LD_URL = ["url", "@id", "mainEntityOfPage", "item", "target", "sameAs"];
function ld(v, s, key) {
  if (Array.isArray(v)) return v.map((x) => ld(x, s, key));
  if (v && typeof v === "object") {
    const o = v;
    for (const k of Object.keys(o)) o[k] = k === "inLanguage" ? s.locale : ld(o[k], s, k);
    return o;
  }
  if (typeof v !== "string" || !key) return v;
  if (LD_TEXT.includes(key)) return s.tr(v) ?? v;
  if (LD_URL.includes(key)) {
    const p = sitePath(s, v);
    return p !== void 0 && urlFor(s, s.locale, p) || v;
  }
  return v;
}
function seo(html, s) {
  const edits = [];
  const raw = (x) => {
    const m = /=\s*(?:"([^"]*)|'([^']*)|([^\s>]*))/.exec(html.slice(x.range[0], x.range[1]));
    return m ? m[1] ?? m[2] ?? m[3] : "";
  };
  const managed = (l) => l.toLowerCase() === "x-default" || s.bases.some(([b2]) => b2.toLowerCase() === l.toLowerCase());
  let canonical, headEnd, ldNext = false;
  for (const t of tokenize(html)) {
    if (t.t === "end") {
      if (t.name === "head" && headEnd === void 0) headEnd = t.range[0];
      continue;
    }
    if (t.t === "text" && ldNext) {
      const src = html.slice(t.range[0], t.range[1]);
      try {
        edits.push([t.range[0], t.range[1], JSON.stringify(ld(JSON.parse(src.trim()), s)).replace(/<\//g, "<\\/")]);
      } catch {
      }
    }
    ldNext = false;
    if (t.t !== "start") continue;
    const a = t.attrs, get2 = (n) => {
      const x = a.find((y) => y.name === n);
      return x && raw(x);
    };
    const mods = /* @__PURE__ */ new Map(), set = (n, v) => mods.set(n, v);
    if (t.name === "html") {
      set("lang", s.locale);
      set("dir", dir(s.locale));
    } else if (t.name === "script") ldNext = get2("type") === "application/ld+json";
    else if (t.name === "link" && get2("rel") !== void 0) {
      const rels = get2("rel").toLowerCase().split(/\s+/), href = get2("href"), lang = get2("hreflang");
      if (rels.includes("canonical")) {
        if (href !== void 0) canonical ??= sitePath(s, href) ?? s.path;
        edits.push([t.range[0], t.range[1], ""]);
        continue;
      }
      if (rels.includes("alternate") && lang !== void 0) {
        if (managed(lang)) {
          edits.push([t.range[0], t.range[1], ""]);
          continue;
        }
        if (href?.startsWith("/") && !href.startsWith("//")) set("href", originOf(base(s, s.source) ?? "") + href);
      }
    } else if (t.name === "meta") {
      const p = get2("property"), c = get2("content");
      if (p === "og:locale") set("content", s.locale.replace(/-/g, "_"));
      else if (p === "og:url" && c !== void 0) {
        const sp = sitePath(s, c), u = sp === void 0 ? void 0 : urlFor(s, s.locale, sp);
        if (u) set("content", u);
      }
    } else if (t.name === "a" && get2("hreflang") === void 0 && get2("data-glot-skip") === void 0) {
      const h = get2("href"), l = h === void 0 ? void 0 : localizeHref(s, h);
      if (l !== void 0) set("href", l);
    }
    if (!mods.size) continue;
    const q = (n, v) => `${n}="${v.replace(/"/g, "&quot;")}"`;
    const parts = a.map((x) => {
      const v = mods.get(x.name);
      if (v === void 0) return html.slice(x.range[0], x.range[1]);
      mods.delete(x.name);
      return q(html.slice(x.range[0], x.range[0] + x.name.length), v);
    });
    for (const [n, v] of mods) parts.push(q(n, v));
    edits.push([t.range[0], t.range[1], `<${html.slice(t.range[0] + 1, t.range[0] + 1 + t.name.length)}${parts.map((x) => " " + x).join("")}${t.selfClosing ? " />" : ">"}`]);
  }
  if (headEnd !== void 0) {
    const p = canonical ?? s.path, c = urlFor(s, s.locale, p);
    const alt = s.bases.map(([l]) => [l, urlFor(s, l, p)]);
    const x = urlFor(s, s.source, p);
    if (x !== void 0) alt.push(["x-default", x]);
    edits.push([headEnd, headEnd, (c ? `<link rel="canonical" href="${esc2(c)}">` : "") + alt.map(([l, u]) => `<link rel="alternate" hreflang="${esc2(l)}" href="${esc2(u)}">`).join("")]);
  }
  edits.sort((x, y) => x[0] - y[0]);
  let out = "", at = 0;
  for (const [a, b2, v] of edits) if (a >= at) {
    out += html.slice(at, a) + v;
    at = b2;
  }
  return out + html.slice(at);
}

// src/express.ts
function fileStore(dir2) {
  const file = (k) => join(dir2, encodeURIComponent(k) + ".json");
  return {
    get: (k) => readFile(file(k), "utf8").catch(() => null),
    async put(k, v) {
      await mkdir(dir2, { recursive: true });
      const tmp = `${file(k)}.${process.pid}.tmp`;
      await writeFile(tmp, v);
      await rename(tmp, file(k));
    }
  };
}
function glotEdge(o) {
  const edge = createEdge({ translate, ...o });
  const send = async (res, out) => {
    const h = {};
    out.headers.forEach((v, k) => h[k] = v);
    const cookies = out.headers.getSetCookie();
    if (cookies.length) h["set-cookie"] = cookies;
    res.writeHead(out.status, h);
    res.end(Buffer.from(await out.arrayBuffer()));
  };
  return async (req, res, next) => {
    try {
      if (req.headers["x-glot-edge-origin"] === "1") return next();
      const host = req.headers.host ?? "localhost";
      const url = new URL(req.url ?? "/", `http://${host}`);
      const headers = new Headers();
      for (const [k, v] of Object.entries(req.headers)) if (typeof v === "string") headers.set(k, v);
      else if (Array.isArray(v)) headers.set(k, v.join(", "));
      if (url.pathname.startsWith("/__glot/")) {
        const body = req.method === "POST" ? await new Promise((ok, no) => {
          let b2 = "";
          req.setEncoding("utf8");
          req.on("data", (c) => {
            if ((b2 += c).length > 65536) req.destroy();
          }).on("end", () => ok(b2)).on("error", no);
        }) : void 0;
        const own = await edge.own(new Request(url, { method: req.method, headers, body }));
        if (own) return void await send(res, own);
      }
      const r = route(url, headers, o);
      if ("redirect" in r) {
        res.writeHead(302, { location: r.redirect, vary: "Accept-Language, Cookie" }).end();
        return;
      }
      if (r.locale === o.source) return next();
      if (req.method !== "GET" && req.method !== "HEAD") {
        req.url = r.path + url.search;
        return next();
      }
      const base2 = o.origin ?? `http://127.0.0.1:${req.socket.localPort}`;
      const originUrl = new URL(r.path + url.search, base2);
      const fwd = new Headers(headers);
      fwd.set("x-glot-edge-origin", "1");
      fwd.set("x-glot-locale", r.locale);
      fwd.delete("accept-encoding");
      const out = await edge(new Request(url, { method: req.method, headers }), r.locale, originUrl, (u) => fetch(u, { method: req.method, headers: fwd, redirect: "manual" }));
      await send(res, out);
    } catch (e) {
      next(e);
    }
  };
}
export {
  fileStore,
  glotEdge
};
