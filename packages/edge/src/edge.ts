// glot Edge runtime, shared by the Cloudflare Worker, the Next proxy and the Express middleware: fetch the origin
// page, translate its rendered HTML from the project's translation memory (TM), serve. No dependencies.
//
// Guarantees:
// - Fail-open: glot slow/down/broken → the origin's HTML, untranslated (`x-glot-edge: fail-open`), never a 5xx from glot.
//   A cold start waits at most `timeout` ms for glot; after that, the last known TM (memory, then `store`) is used
//   and refreshed in the background (stale-while-revalidate / stale-if-error).
// - One version per page: each request takes ONE immutable TM snapshot (locale@version); a page never mixes two
//   versions. The page cache key contains the version, so a version bump in glot *is* the purge.
// - Kill switch: `GET /edge/v1/{project}/status` (≤ `statusTtl`, 20 s by default) says whether Edge is on for the
//   project / this host; off → passthrough to the origin in < 1 min, no redeploy.
// - Discovery: untranslated segments are reported (hash/text/context + URL path, never query string, cookies or
//   anything about the visitor), only for anonymous requests, deduplicated per isolate, batched in `waitUntil`.
// - Pages behind sign-in (a session cookie or `Authorization`), status `auth.pages`: `server` (default) =
//   translated from the TM for this response only — `Cache-Control: private`, `Vary: Cookie`, never in the page
//   cache, never discovered, nothing about the page sent to glot; `patcher` / `off` = the origin's page untouched
//   (the site's own `<script data-glot-app>` tag translates it in the browser, or nobody does). The request
//   (cookies included) reaches the origin as is and its `Set-Cookie` comes back as is: same domain, same session.
// - Application paths (status `app`: login, account, API…): never translated, cached or discovered; the runtimes
//   call `app()` first: a redirect to the source URL (locale as the site expects it) or the origin untouched.
// - `Server-Timing: glot;dur=…` = time spent in glot code (TM + rewrite), `origin;dur=…` = the origin fetch.

import { classify, decide, type AppRules } from "./app-paths.ts";

/** TM snapshot, wire format of `GET /edge/v1/{project}/{locale}/tm` (glot_core::edge::tm): segment hash →
 * one translation valid in any context, or per context (`<role>|<page template>`, `*` = catalog default). */
export interface Tm {
  v: number;
  locale: string;
  t: Readonly<Record<string, string | Readonly<Record<string, string>>>>;
}
/** Exact context, then the catalog default; an ambiguous text in a new context has no translation. */
export function lookup(tm: Tm, hash: string, context: string): string | undefined {
  const e = tm.t[hash];
  return typeof e === "string" ? e : e?.[context] ?? e?.["*"];
}
export interface Status {
  enabled: boolean;
  /** Hosts where Edge is switched off (per-domain kill switch). */
  off: string[];
  /** Current TM version per locale. */
  versions: Record<string, number>;
  /** Project CSS selectors never extracted (user content, widgets). */
  selectors?: { include: string[]; exclude: string[] };
  /** Locales ready to be listed (hreflang), from `/cdn/{project}/locales.json`; undefined = unknown (all listed). */
  ready?: string[];
  /** Application paths (login, account, API…) kept on the origin's own URL (app-paths.ts). */
  app?: AppRules;
  /** Requests with a session: how they are served (default `server`) + app-mode patcher discovery. */
  auth?: { pages?: "off" | "patcher" | "server"; discover?: boolean };
}
/** A segment glot has no translation for (from the segmenter); `discoverable: false` ones are never sent. */
export interface Missing {
  hash: string;
  text: string;
  /** Role of the carrying element (`button`, `a`, `h1`, `title`…): the server derives the context from it. */
  role?: string;
  discoverable?: boolean;
}
export interface TranslateResult {
  html: string;
  /** Rendered segments / translated ones (coverage of this page). */
  total: number;
  translated: number;
  missing: Missing[];
  /** Every rendered segment (per-URL coverage), optional. */
  seen?: { hash: string; role: string }[];
}
export interface TranslateCtx {
  /** Public (localized) URL of the page. */
  url: URL;
  /** Origin path of the page (source terms, no locale prefix, no query). */
  path: string;
  locale: string;
  source: string;
  /** Locales listed as hreflang alternates: the ready ones + this page's. */
  locales: readonly string[];
  /** Selectors never extracted. */
  skip: string[];
  /** Client discovery on (project key set, anonymous visitor): the patcher tag gets `data-missing`. */
  discover?: boolean;
}
/** Synchronous: one TM snapshot in, one page out (translate.ts). */
export type Translator = (html: string, tm: Tm, ctx: TranslateCtx) => TranslateResult;

/** Persistent last-known state (Worker: Cache API; Node: a directory). */
export interface Store {
  get(key: string): Promise<string | null>;
  put(key: string, value: string): Promise<void>;
}
/** Shared page cache (Worker: `caches.default`). Keys already contain locale and version. */
export interface PageCache {
  match(key: string): Promise<Response | undefined>;
  put(key: string, res: Response): Promise<void>;
}

export interface EdgeOptions {
  /** glot server, e.g. `https://api.glotlang.com`. */
  server: string;
  project: string;
  /** Project public key (`pk_…`): sent with discovery reports. No key → no discovery. */
  key?: string;
  source: string;
  /** Locales served besides the source (hreflang alternates). */
  locales?: readonly string[];
  translate: Translator;
  /** Budget for glot on a cold start, ms (default 300). */
  timeout?: number;
  /** Status revalidation period, ms (default 20 000: kill switch < 1 min including the server's cache). */
  statusTtl?: number;
  /** Larger HTML pages are streamed through untranslated (default 3 MB). */
  maxHtml?: number;
  store?: Store;
  pageCache?: PageCache;
  /** Seconds a translated page is kept in `pageCache` when the origin doesn't say (default 0 = obey the origin only). */
  pageTtl?: number;
  fetch?: typeof fetch;
  now?: () => number;
}

type Wait = (p: Promise<unknown>) => void;
type State = "translated" | "source" | "off" | "quota" | "fail-open" | "passthrough" | "auth";

export const SESSION_COOKIE = /(?:^|;\s*)[^=;]*(sess|auth|token|sid|jwt|login|logged|user|account)[^=;]*=/i;
const HOP = ["content-length", "content-encoding", "transfer-encoding", "connection", "keep-alive", "etag"];

export function createEdge(o: EdgeOptions) {
  const base = o.server.replace(/\/+$/, "");
  const f = o.fetch ?? ((...a: Parameters<typeof fetch>) => globalThis.fetch(...a));
  const now = o.now ?? Date.now;
  const timeout = o.timeout ?? 300;
  const statusTtl = o.statusTtl ?? 20_000;
  const maxHtml = o.maxHtml ?? 3_000_000;
  const proj = encodeURIComponent(o.project);

  let status: { at: number; v: Status } | undefined;
  let statusRun: Promise<Status | undefined> | undefined;
  const tms = new Map<string, Tm>();
  const tmRun = new Map<string, Promise<Tm | undefined>>();
  let restored: Promise<void> | undefined;
  const seen = new Set<string>();
  // Plan usage (`edge_pages`): counted here, flushed every 100 pages / 30 s; a 402 = origin untranslated for 5 min.
  let served = 0, flushedAt = now(), quotaUntil = 0;
  const count = (wait?: Wait) => {
    served++;
    if (!o.key || !wait || (served < 100 && now() - flushedAt < 30_000)) return;
    const pages = served;
    served = 0;
    flushedAt = now();
    wait(
      f(`${base}/edge/v1/${proj}/usage`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ key: o.key, pages }) })
        .then((r) => void (r.status === 402 && (quotaUntil = now() + 300_000)))
        .catch(() => {}),
    );
  };

  const get = async (url: string, ms: number) => {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), ms);
    try {
      return await f(url, { signal: ctl.signal, headers: { accept: "application/json" } });
    } finally {
      clearTimeout(t);
    }
  };

  // Last known state from the persistent store, once per isolate (cheap and local: before any network).
  const restore = () =>
    (restored ??= (async () => {
      if (!o.store) return;
      try {
        const s = await o.store.get(`status`);
        if (s && !status) status = { at: 0, v: JSON.parse(s) }; // at 0 = stale: revalidated on first use
      } catch {
        /* corrupt / unavailable store: start empty */
      }
    })());

  const refreshStatus = () =>
    (statusRun ??= (async () => {
      try {
        const res = await get(`${base}/edge/v1/${proj}/status`, 5000);
        if (!res.ok) throw new Error(String(res.status));
        const v = (await res.json()) as Status;
        // Ready locales (hreflang only lists those); unreachable manifest → the last known list.
        const ready = await get(`${base}/cdn/${proj}/locales.json`, 5000)
          .then((r) => (r.ok ? r.json() : undefined))
          .then((m: { locales?: { code: string; ready?: boolean }[] } | undefined) => m?.locales?.filter((l) => l.ready !== false).map((l) => l.code))
          .catch(() => undefined);
        status = { at: now(), v: { enabled: v.enabled !== false, off: v.off ?? [], versions: v.versions ?? {}, selectors: v.selectors, ready: ready ?? status?.v.ready, app: v.app, auth: v.auth } };
        void o.store?.put("status", JSON.stringify(status.v)).catch(() => {});
        // Prefetch newer TMs of the locales we serve: the next request gets the new version, whole.
        await Promise.all([...tms.values()].filter((t) => (status!.v.versions[t.locale] ?? 0) > t.v).map((t) => loadTm(t.locale, status!.v.versions[t.locale])));
        return status.v;
      } catch {
        if (status) status.at = now() - statusTtl / 2; // stale-if-error: keep it, retry in half a period
        return status?.v;
      } finally {
        statusRun = undefined;
      }
    })());

  const loadTm = (locale: string, want: number) => {
    const key = `${locale}@${want}`;
    const running = tmRun.get(key);
    if (running) return running;
    const p = (async () => {
      const prev = tms.get(locale) ?? (await fromStore(locale));
      if (prev && prev.v >= want) return prev;
      try {
        // ponytail: full snapshot each version (fetched only when `status` says it changed); deltas if TMs get big.
        const res = await get(`${base}/edge/v1/${proj}/${encodeURIComponent(locale)}/tm`, 10_000);
        if (!res.ok) throw new Error(String(res.status));
        const body = (await res.json()) as Tm;
        if (typeof body?.t !== "object" || body.t === null) throw new Error("bad tm");
        // A NEW frozen object per version: snapshots held by in-flight requests never change.
        const tm: Tm = Object.freeze({ v: Number(body.v) || 0, locale, t: Object.freeze(body.t) });
        if (!prev || tm.v >= prev.v) {
          tms.set(locale, tm);
          void o.store?.put(`tm:${locale}`, JSON.stringify(tm)).catch(() => {});
        }
        return tms.get(locale);
      } catch {
        return prev; // stale-if-error
      } finally {
        tmRun.delete(key);
      }
    })();
    tmRun.set(key, p);
    return p;
  };

  const fromStore = async (locale: string): Promise<Tm | undefined> => {
    try {
      const s = await o.store?.get(`tm:${locale}`);
      if (!s) return undefined;
      const tm = JSON.parse(s) as Tm;
      if (!tms.has(locale)) tms.set(locale, tm);
      return tms.get(locale);
    } catch {
      return undefined;
    }
  };

  /** TM snapshot to use for this request, or why there is none. Never waits more than `timeout` on glot. */
  async function snapshot(locale: string, host: string, wait?: Wait): Promise<Tm | "off" | undefined> {
    await restore();
    const fresh = status && now() - status.at < statusTtl;
    let st = status?.v;
    if (!fresh) {
      const p = refreshStatus();
      if (st) wait?.(p); // stale-while-revalidate
      else {
        wait?.(p);
        st = await race(p, timeout);
      }
    }
    if (st && (!st.enabled || st.off.includes(host))) return "off";
    const want = st?.versions[locale] ?? 0;
    const have = tms.get(locale) ?? (await fromStore(locale));
    if (have && have.v >= want) return have;
    const p = loadTm(locale, want);
    if (have) {
      wait?.(p); // older version: serve it whole now, the new one from the next request
      return have;
    }
    wait?.(p); // cold: if it misses the budget, it still lands for the next request
    return race(p, timeout);
  }

  /** Body of `POST /edge/v1/{project}/missing` (crates/server/src/edge/missing.rs). */
  function report(locale: string, out: TranslateResult, url: URL, wait?: Wait) {
    if (!o.key || !wait) return;
    const path = url.pathname; // never the query string: it can carry visitor data
    if (seen.size > 20_000) seen.clear(); // ponytail: bounded memory, the server dedupes too
    const segments = out.missing
      .filter((m) => m.discoverable !== false && !seen.has(`${locale}|${m.hash}|${m.role ?? ""}`))
      .slice(0, 200)
      .map((m) => (seen.add(`${locale}|${m.hash}|${m.role ?? ""}`), { hash: m.hash, text: m.text, role: m.role ?? "", url: path }));
    // Which segments this page renders (per-URL coverage): once per path, locale and isolate.
    const pageKey = `page|${locale}|${path}`;
    const pages = out.seen && !seen.has(pageKey) ? [{ url: path, seen: out.seen.slice(0, 500) }] : [];
    if (pages.length) seen.add(pageKey);
    if (!segments.length && !pages.length) return;
    wait(
      f(`${base}/edge/v1/${proj}/missing`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ key: o.key, locale, auth: false, segments, pages }),
      }).catch(() => {}),
    );
  }

  /**
   * Serve `req` (whose URL is the public, localized URL) in `locale`, `origin(path)` fetching the origin page.
   * `path` = origin path + search (locale prefix already removed by the router).
   */
  let patcher: { at: number; js: string } | undefined;
  /**
   * glot's own same-origin paths (so the origin's CSP `script-src 'self'` allows them): `/__glot/tm.json?locale=es`
   * = the TM snapshot this runtime serves (the client patcher re-applies it after hydration), `/__glot/patcher.js`
   * = the patcher bundle (from glot, cached 10 min, stale-if-error). Anything else: undefined.
   */
  async function own(req: Request, wait?: Wait): Promise<Response | undefined> {
    const u = new URL(req.url);
    if (u.pathname === "/__glot/tm.json") {
      const l = u.searchParams.get("locale") ?? "";
      const tm = l && l !== o.source ? await snapshot(l, u.hostname, wait) : undefined;
      if (!tm || tm === "off") return new Response("{}", { status: 404, headers: { "content-type": "application/json", "cache-control": "max-age=30" } });
      // `since=v`: the patcher polling after a discovery report — 204 until a newer version exists (cheap).
      const since = Number(u.searchParams.get("since") ?? -1);
      if (tm.v <= since) return new Response(null, { status: 204, headers: { "cache-control": "max-age=10" } });
      const skip = status?.v.selectors?.exclude;
      return new Response(JSON.stringify(skip?.length ? { ...tm, skip } : tm), { headers: { "content-type": "application/json; charset=utf-8", "cache-control": "public, max-age=30, stale-if-error=86400" } });
    }
    if (u.pathname === "/__glot/missing") return missing(req, wait);
    if (u.pathname === "/__glot/patcher.js") {
      if (!patcher || now() - patcher.at > 600_000) {
        const js = await get(`${base}/edge/patcher.js`, 3000).then((r) => (r.ok ? r.text() : Promise.reject()), () => undefined).catch(() => undefined);
        patcher = { at: now(), js: js ?? patcher?.js ?? "" };
      }
      return new Response(patcher.js, { headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=600" } });
    }
    return undefined;
  }

  /**
   * `POST /__glot/missing` from the client patcher: forwarded to `/edge/v1/{project}/missing` with the project key
   * and `auth` decided HERE from the visitor's request (HttpOnly session cookies included). Only hashes, texts,
   * roles and paths go through (re-validated, capped); the server rechecks hashes and personal data.
   */
  async function missing(req: Request, wait?: Wait): Promise<Response> {
    const done = new Response(null, { status: 204 });
    if (!o.key || req.method !== "POST") return done;
    let b: { locale?: unknown; segments?: unknown };
    try {
      const raw = await req.text();
      if (raw.length > 65_536) return done;
      b = JSON.parse(raw);
    } catch {
      return done;
    }
    const str = (x: unknown, max: number) => (typeof x === "string" && x.length <= max ? x : undefined);
    const locale = str(b.locale, 35);
    if (!locale || !/^[A-Za-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(locale) || locale === o.source || !Array.isArray(b.segments)) return done;
    const segments = b.segments.slice(0, 200).flatMap((x: Record<string, unknown>) => {
      // source-terms path, like the server-side reports: no query/fragment, no `/{locale}` prefix
      const hash = str(x?.hash, 64), text = str(x?.text, 2000), role = str(x?.role, 64) ?? "";
      const path = str(x?.url, 2000)?.split(/[?#]/)[0];
      if (!hash || !text || !path?.startsWith("/")) return [];
      return [{ hash, text, role, url: path.replace(new RegExp(`^/${locale}(?=/|$)`, "i"), "") || "/" }];
    });
    if (!segments.length) return done;
    const p = f(`${base}/edge/v1/${proj}/missing`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key: o.key, locale, auth: !anonymous(req.headers), segments }),
    }).catch(() => {});
    if (wait) wait(p);
    else await p;
    return done;
  }

  /** Current application-path rules (status, within `statusTtl`; cold: at most `timeout` ms). */
  async function appRules(wait?: Wait): Promise<AppRules | undefined> {
    await restore();
    if (!status || now() - status.at >= statusTtl) {
      const p = refreshStatus();
      wait?.(p);
      if (!status) await race(p, timeout);
    }
    return status?.v.app;
  }

  /** `path` (origin path, no query) is an application path: never routed to a locale. */
  async function isApp(path: string, wait?: Wait): Promise<boolean> {
    const a = await appRules(wait);
    return !!a?.rules?.length && classify(a.rules, a.action ?? "redirect", path) !== undefined;
  }

  /**
   * Application path decision for `req` served in `locale` (`path` = origin path, locale prefix removed):
   * a redirect Response to the source URL with the locale as the site expects it, `"proxy"` (serve the origin
   * untouched: no translation, cache or discovery), or undefined (a normal page). Fail-open: rules unknown → undefined.
   */
  async function app(req: Request, locale: string, path: string, wait?: Wait): Promise<Response | "proxy" | undefined> {
    const a = await appRules(wait);
    if (!a?.rules?.length) return undefined;
    const u = new URL(req.url);
    const d = decide({ rules: a.rules, action: a.action ?? "redirect", locale_style: a.locale ?? "none", locale, origin: u.origin, host: u.host, req_path: u.pathname, path, query: u.search.slice(1), method: req.method, cookie: req.headers.get("cookie") ?? "" });
    if (!d || d.kind === "proxy") return d && "proxy";
    return new Response(null, { status: d.status, headers: { location: d.location, "set-cookie": d.cookie, "cache-control": "no-store", "x-glot-edge": "app-redirect" } });
  }

  /**
   * A JavaScript chunk, one URL for every language (subdirectory mode): served in `locale` (the page's, from its
   * `Referer`) with its hard-coded strings translated, so client components hydrate on the translated HTML
   * (no React #418). The rewrite is glot's (`POST /edge/v1/{project}/{locale}/bundle`, a real JS parser), cached
   * per (URL, locale, version) in `pageCache`. While Edge is on, no chunk is `immutable` in the browser (the same
   * URL changes with the language and the version): `public, no-cache` + ETag, revalidated with a 304.
   * Fail-open: the origin chunk, `no-store` (the next load retries).
   */
  async function bundle(req: Request, locale: string, origin: (r: Request) => Promise<Response>, wait?: Wait): Promise<Response> {
    const t0 = now();
    const js = (r: Response) => r.status === 200 && /javascript|ecmascript/.test(r.headers.get("content-type") ?? "");
    const revalidate = (r: Response) => {
      const res = new Response(r.body, r);
      res.headers.set("cache-control", "public, no-cache");
      res.headers.delete("expires");
      return res;
    };
    const tm = locale === o.source || req.method !== "GET" ? undefined : await snapshot(locale, new URL(req.url).hostname, wait).catch(() => undefined);
    if (!tm || tm === "off" || !o.key) {
      const r = await origin(req);
      if (!js(r) && r.status !== 304) return r;
      if (locale === o.source || tm === "off" || r.status === 304) return revalidate(r);
      const res = revalidate(r); // glot unreachable on a localized page: the source chunk, kept nowhere
      for (const h of ["etag", "last-modified"]) res.headers.delete(h);
      res.headers.set("cache-control", "no-store");
      return timing(res, "fail-open", now() - t0, 0);
    }
    const key = `https://glot-edge.invalid/_bundle/${locale}.${tm.v}${new URL(req.url).pathname}${new URL(req.url).search}`;
    const fresh = (r: Response) => (req.headers.get("if-none-match") === r.headers.get("etag") && r.headers.get("etag") ? new Response(null, { status: 304, headers: r.headers }) : r);
    const hit = await o.pageCache?.match(key).catch(() => undefined);
    if (hit) return timing(fresh(revalidate(hit)), "translated", now() - t0, 0, "hit");
    // The browser's validators are for whichever language it saw last: never let the origin answer 304 here.
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
    const timer = setTimeout(() => ctl.abort(), Math.max(timeout, 2000));
    try {
      const g = await f(`${base}/edge/v1/${proj}/${encodeURIComponent(locale)}/bundle`, { method: "POST", signal: ctl.signal, headers: { "content-type": "text/javascript", "x-glot-key": o.key }, body });
      if (!g.ok) return failOpen();
      const headers = new Headers(page.headers);
      for (const h of [...HOP, "last-modified", "expires"]) headers.delete(h);
      headers.set("cache-control", "public, no-cache");
      for (const h of ["etag", "x-glot-bundle", "x-glot-version"]) { const v = g.headers.get(h); if (v) headers.set(h, v); }
      const out = await g.arrayBuffer();
      if (o.pageCache) {
        const copy = new Response(out, { headers });
        copy.headers.set("cache-control", "public, max-age=31536000"); // the key holds locale + version
        wait?.(o.pageCache.put(key, copy).catch(() => {}));
      }
      return timing(fresh(new Response(out, { status: 200, headers })), "translated", now() - t0 - tOrigin, tOrigin, "miss");
    } catch {
      return failOpen();
    } finally {
      clearTimeout(timer);
    }
  }

  return Object.assign(serve, { own, app, isApp, bundle });

  async function serve(req: Request, locale: string, originUrl: URL, origin: (u: URL) => Promise<Response>, wait?: Wait): Promise<Response> {
    const url = new URL(req.url);
    const t0 = now();
    const translatable = locale !== o.source && (req.method === "GET" || req.method === "HEAD");
    // TM and origin in parallel (unless a page cache could answer first); glot's part is bounded by `timeout`.
    const early = translatable && !o.pageCache ? settle(origin(originUrl)) : undefined;
    const tmP = translatable ? snapshot(locale, url.hostname, wait).catch(() => undefined) : Promise.resolve(undefined);
    const anon = anonymous(req.headers);

    const tm = await tmP;
    const tGlot = now() - t0;
    const cacheKey = tm && tm !== "off" && anon && o.pageCache ? pageKey(originUrl, url.hostname, tm) : undefined;
    if (cacheKey) {
      const hit = await o.pageCache!.match(cacheKey).catch(() => undefined);
      if (hit) return count(wait), timing(new Response(hit.body, hit), hit.headers.get("x-glot-coverage")?.startsWith("0/") ? "source" : "translated", tGlot, 0, "hit");
    }
    const got = await (early ?? settle(origin(originUrl)));
    if (!("ok" in got)) return new Response("Bad gateway", { status: 502, headers: { "x-glot-edge": "origin-error" } }); // the origin's failure, not glot's
    const page = got.ok;
    const tOrigin = got.ms;
    if (!translatable) return timing(page, "passthrough", 0, tOrigin);
    // Signed in, project says "not by the Edge": the origin's page, untouched, never shared.
    if (!anon && tm !== "off" && (status?.v.auth?.pages ?? "server") !== "server") return timing(personal(new Response(page.body, page)), "auth", tGlot, tOrigin);
    if (tm === "off") return timing(sourceUnderLocale(page), "off", tGlot, tOrigin);
    if (quotaUntil > now()) return timing(sourceUnderLocale(page), "quota", tGlot, tOrigin);
    const type = page.headers.get("content-type") ?? "";
    const len = Number(page.headers.get("content-length") ?? 0);
    if (!type.includes("text/html") || page.status !== 200 || len > maxHtml || req.method === "HEAD" || !page.body)
      return timing(page, "passthrough", tGlot, tOrigin);
    if (!tm) return timing(sourceUnderLocale(page), "fail-open", tGlot, tOrigin);

    let html: string;
    try {
      html = await page.text(); // ponytail: buffered (pages without content-length are checked after reading)
    } catch {
      return new Response("Bad gateway", { status: 502, headers: { "x-glot-edge": "origin-error" } });
    }
    if (html.length > maxHtml) return timing(sourceUnderLocale(new Response(html, page)), "passthrough", tGlot, tOrigin);
    const tT = now();
    let out: TranslateResult;
    try {
      const ready = status?.v.ready;
      const locales = (o.locales ?? []).filter((l) => l === locale || !ready || ready.includes(l));
      if (!locales.includes(locale)) locales.push(locale);
      const ctx: TranslateCtx = { url, path: originUrl.pathname, locale, source: o.source, locales, skip: status?.v.selectors?.exclude ?? [], discover: !!o.key && anon };
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
    if (!anon) personal(res); // translated for this visitor only
    const ttl = cacheKey ? cacheTtl(page.headers, o.pageTtl ?? 0) : 0;
    if (cacheKey && ttl > 0) {
      const copy = res.clone();
      copy.headers.set("cache-control", `public, max-age=${ttl}`);
      wait?.(o.pageCache!.put(cacheKey, copy).catch(() => {}));
    }
    count(wait);
    return timing(res, out.translated ? "translated" : "source", tGlot + now() - tT, tOrigin, "miss");
  };
}

const anonymous = (h: Headers) => !h.has("authorization") && !SESSION_COOKIE.test(h.get("cookie") ?? "");

function pageKey(u: URL, host: string, tm: Tm) {
  const k = new URL(`https://glot-edge.invalid/${host}${u.pathname}`);
  k.search = u.search;
  k.searchParams.set("__glot", `${tm.locale}.${tm.v}`);
  return k.toString();
}

/** Seconds the origin allows a shared cache to keep this page (0 = not shareable). */
export function cacheTtl(h: Headers, fallback: number): number {
  if (h.has("set-cookie")) return 0;
  const cc = (h.get("cache-control") ?? "").toLowerCase();
  if (/private|no-store|no-cache/.test(cc)) return 0;
  const m = /s-maxage=(\d+)/.exec(cc) ?? /max-age=(\d+)/.exec(cc);
  return m ? Number(m[1]) : fallback;
}

/** A response built for one signed-in visitor: a browser may keep it, a shared cache (CDN, proxy) never. */
function personal(res: Response) {
  res.headers.set("cache-control", "private, no-cache");
  res.headers.append("vary", "Cookie");
  return res;
}

/** The source page served under a localized URL (glot off / not ready): never cached as if it were translated. */
function sourceUnderLocale(page: Response) {
  const res = new Response(page.body, page);
  res.headers.set("cache-control", "no-store");
  return res;
}

function timing(res: Response, state: State, glot: number, origin: number, cache?: "hit" | "miss") {
  const r = new Response(res.body, res); // mutable headers
  r.headers.set("x-glot-edge", cache === "hit" ? `${state}; cache=hit` : state);
  r.headers.append("server-timing", `glot;dur=${glot.toFixed(1)};desc="${state}${cache ? ` ${cache}` : ""}", origin;dur=${origin.toFixed(1)}`);
  return r;
}

/** Never rejects; `ms` = how long the origin took. */
function settle(p: Promise<Response>) {
  const t = Date.now();
  return p.then((ok) => ({ ok, ms: Date.now() - t }), (err: unknown) => ({ err, ms: Date.now() - t }));
}

function race<T>(p: Promise<T>, ms: number): Promise<T | undefined> {
  return new Promise((resolve) => {
    const t = setTimeout(() => resolve(undefined), ms);
    p.then(
      (v) => (clearTimeout(t), resolve(v)),
      () => (clearTimeout(t), resolve(undefined)),
    );
  });
}
