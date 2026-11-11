// Cloudflare Worker in front of a site (route it on your zone; `fetch(req)` reaches the origin).
// `/es/pricing` = the origin's `/pricing`, its rendered HTML translated by glot Edge (no keys needed: segments of
// the page are looked up in the project's translation memory; unknown ones are reported, translated by glot and
// served from the next request; meanwhile the source text is served, never a hole). Elements with
// `data-glot="key"` still get the glot catalog value on top (keys win over segments).
//   wrangler.toml: main = "node_modules/@glot/edge/dist/worker.js"
//   [vars] GLOT_SERVER = "https://api.glotlang.com"  GLOT_PROJECT = "site"  LOCALES = "es,de"  SOURCE_LOCALE = "en"
//          GLOT_KEY = "pk_…"   (discovery of new segments + copy experiments)
//          GLOT_EDGE = "0"     (optional: catalogs/`data-glot` only, no segment translation)
//          GLOT_TIMEOUT_MS = "300"  (cold-start budget for glot; past it the origin page is served untranslated)
// Fail-open: whatever happens to glot, the visitor gets the origin's page. Last known TM: isolate memory + Cache API.
// ponytail: experiments overlay + data-glot are plain strings (no ICU formatting).
import { createOnDemandLoader } from "@glot/runtime/ondemand";
import { route } from "./route.ts";
import { glotExperiments, type Decision } from "./experiments.ts";
import { createEdge, type PageCache, type Store } from "./edge.ts";
import { translate } from "./translate.ts";

export interface Env {
  GLOT_SERVER: string;
  GLOT_PROJECT: string;
  /** Comma-separated locales served under `/{locale}/` (glot's on-demand whitelist). */
  LOCALES: string;
  SOURCE_LOCALE?: string;
  /** Project public key (`pk_…`): segment discovery + segmented copy experiments. */
  GLOT_KEY?: string;
  GLOT_EDGE?: string;
  GLOT_TIMEOUT_MS?: string;
}

interface Element {
  getAttribute(name: string): string | null;
  setAttribute(name: string, value: string): void;
  setInnerContent(content: string): void;
}
declare const HTMLRewriter: new () => {
  on(selector: string, handlers: { element(e: Element): void }): InstanceType<typeof HTMLRewriter>;
  transform(res: Response): Response;
};
declare const caches: { default?: { match(k: string): Promise<Response | undefined>; put(k: string, r: Response): Promise<void> } } | undefined;

let load: ReturnType<typeof createOnDemandLoader> | undefined; // per isolate: memory cache + ETag revalidation
let xp: ReturnType<typeof glotExperiments> | undefined;
let edge: ReturnType<typeof createEdge> | undefined;

/** Cache API as the Worker's "disk": last known TM/status survive isolate restarts (per colo). */
const cacheStore = (c: NonNullable<NonNullable<typeof caches>["default"]>): Store => ({
  get: async (k) => (await c.match(`https://glot-edge.invalid/_state/${k}`))?.text() ?? null,
  put: (k, v) => c.put(`https://glot-edge.invalid/_state/${k}`, new Response(v, { headers: { "cache-control": "max-age=2592000" } })),
});

function getEdge(env: Env, source: string, locales: string[]) {
  const c = typeof caches !== "undefined" ? caches?.default : undefined;
  return (edge ??= createEdge({
    server: env.GLOT_SERVER,
    project: env.GLOT_PROJECT,
    key: env.GLOT_KEY,
    source,
    locales,
    translate,
    timeout: Number(env.GLOT_TIMEOUT_MS) || 300,
    store: c && cacheStore(c),
    pageCache: c as PageCache | undefined,
  }));
}

/** Language of the same-site page that requested this subresource (`Referer`); none or foreign → the source. */
// ponytail: `Referrer-Policy: no-referrer`/`origin` on the site hides the path → source chunks (the patcher still
// re-applies the TM, React logs its recoverable #418 as before).
export function refererLocale(req: Request, url: URL, o: { source: string; locales: string[] }): string {
  try {
    const ref = new URL(req.headers.get("referer") ?? "");
    if (ref.host !== url.host) return o.source;
    const r = route(ref, new Headers(), o);
    return "locale" in r ? r.locale : o.source;
  } catch {
    return o.source;
  }
}

export default {
  async fetch(req: Request, env: Env, ctx?: { waitUntil(p: Promise<unknown>): void }): Promise<Response> {
    const url = new URL(req.url);
    const source = env.SOURCE_LOCALE ?? "en";
    const locales = env.LOCALES.split(",").map((l) => l.trim());
    const wait = ctx && ((p: Promise<unknown>) => ctx.waitUntil(p));
    if (env.GLOT_EDGE !== "0" && url.pathname.startsWith("/__glot/")) {
      const own = await getEdge(env, source, locales).own(req, wait);
      if (own) return own;
    }
    const edgeOn = env.GLOT_EDGE !== "0";
    // JavaScript chunks (same URL for every language): localized for the language of the page that loads them.
    if (edgeOn && /\.m?js$/.test(url.pathname)) {
      const page = refererLocale(req, url, { source, locales });
      return getEdge(env, source, locales).bundle(req, page, (r) => fetch(r), wait);
    }
    let r = route(url, req.headers, { source, locales });
    // Application paths (login, account, API…) are never sent to a locale URL…
    if ("redirect" in r && edgeOn && (await getEdge(env, source, locales).isApp(url.pathname, wait))) r = { locale: source, path: url.pathname };
    if ("redirect" in r) return new Response(null, { status: 302, headers: { location: r.redirect, vary: "Accept-Language, Cookie" } });
    // …and under one: back to the source URL, or the origin's response untouched (no translation, cache, discovery).
    if (edgeOn && r.locale !== source) {
      const a = await getEdge(env, source, locales).app(req, r.locale, r.path, wait);
      if (a === "proxy") {
        const u = new URL(url);
        u.pathname = r.path;
        return fetch(new Request(u, req));
      }
      if (a) return a;
    }

    let d: Decision | undefined;
    if (env.GLOT_KEY && req.method === "GET" && req.headers.get("accept")?.includes("text/html")) {
      xp ??= glotExperiments({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT, key: env.GLOT_KEY });
      d = await xp(req, r.locale, wait).catch(() => undefined);
    }
    const originUrl = new URL(url);
    originUrl.pathname = r.path;
    const origin = (u: URL) => {
      const up = new Request(u, req);
      up.headers.delete("x-glot-xp"); // client-forged
      if (d?.header) up.headers.set("x-glot-xp", d.header);
      return fetch(up);
    };
    const useEdge = edgeOn && r.locale !== source;
    let page: Response;
    if (useEdge) page = await getEdge(env, source, locales)(req, r.locale, originUrl, origin, wait);
    else page = await origin(originUrl);
    if (!page.headers.get("content-type")?.includes("text/html")) return page;

    let messages: Record<string, unknown> | null = null;
    if (r.locale !== source) {
      load ??= createOnDemandLoader({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT });
      // Fail-open: the catalog is optional on top of segments; never wait long for it.
      messages = await Promise.race([load(r.locale).catch(() => null), new Promise<null>((ok) => setTimeout(() => ok(null), 300))]);
    }
    const res = new Response(page.body, page);
    if (d?.header) res.headers.set("cache-control", "private, no-store"); // a variant is never shared
    else if (!useEdge && r.locale !== source && !messages) res.headers.set("cache-control", "no-store"); // never cache the source under the German URL
    for (const c of d?.cookies ?? []) res.headers.append("set-cookie", c);
    const overlay = d?.overlay ?? {};
    const lang = r.locale !== source;
    if (!messages && !Object.keys(overlay).length && !lang) return res;
    const lookup = (key: string) => overlay[key] ?? key.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown> | undefined)?.[k], messages);
    try {
      return new HTMLRewriter()
        .on("html", { element: (e) => lang && e.setAttribute("lang", r.locale) })
        .on("[data-glot]", {
          element(e) {
            const v = lookup(e.getAttribute("data-glot") ?? "");
            if (typeof v === "string") e.setInnerContent(v); // text, HTML-escaped by HTMLRewriter
          },
        })
        .transform(res);
    } catch {
      return res; // fail-open
    }
  },
};
