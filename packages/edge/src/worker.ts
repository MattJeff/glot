// Cloudflare Worker in front of a static / marketing site (route it on your zone; `fetch(req)` reaches the origin).
// `/de/pricing` = the origin's `/pricing` with every `data-glot="key"` element filled from glot's `de` catalog,
// asked on demand: while glot translates (first visit), the source page is served uncached.
//   wrangler.toml: main = "node_modules/@glot/edge/dist/worker.js"
//   [vars] GLOT_SERVER = "https://glot.example.com"  GLOT_PROJECT = "site"  LOCALES = "de,it,ja"  SOURCE_LOCALE = "en"
// Copy experiments: add `GLOT_KEY = "pk_…"` → arms decided here, `data-glot` elements get the variant text,
// on source pages too (static sites have no React to read `x-glot-xp`; it's still forwarded to the origin).
// ponytail: plain strings only (no ICU formatting) and links inside the page are not rewritten to `/de/…`.
import { createOnDemandLoader } from "@glot/runtime/ondemand";
import { route } from "./route.ts";
import { glotExperiments, type Decision } from "./experiments.ts";

export interface Env {
  GLOT_SERVER: string;
  GLOT_PROJECT: string;
  /** Comma-separated locales served under `/{locale}/` (glot's on-demand whitelist). */
  LOCALES: string;
  SOURCE_LOCALE?: string;
  /** Project public key (`pk_…`): enables segmented copy experiments. */
  GLOT_KEY?: string;
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

let load: ReturnType<typeof createOnDemandLoader> | undefined; // per isolate: memory cache + ETag revalidation
let xp: ReturnType<typeof glotExperiments> | undefined;

export default {
  async fetch(req: Request, env: Env, ctx?: { waitUntil(p: Promise<unknown>): void }): Promise<Response> {
    const url = new URL(req.url);
    const source = env.SOURCE_LOCALE ?? "en";
    const r = route(url, req.headers, { source, locales: env.LOCALES.split(",").map((l) => l.trim()) });
    if ("redirect" in r) return new Response(null, { status: 302, headers: { location: r.redirect, vary: "Accept-Language, Cookie" } });

    let d: Decision | undefined;
    if (env.GLOT_KEY && req.method === "GET" && req.headers.get("accept")?.includes("text/html")) {
      xp ??= glotExperiments({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT, key: env.GLOT_KEY });
      d = await xp(req, r.locale, ctx && ((p) => ctx.waitUntil(p)));
    }
    url.pathname = r.path;
    const up = new Request(url, req);
    up.headers.delete("x-glot-xp"); // client-forged
    if (d?.header) up.headers.set("x-glot-xp", d.header);
    const page = await fetch(up);
    if (!page.headers.get("content-type")?.includes("text/html")) return page;
    let messages: Record<string, unknown> | null = null;
    if (r.locale !== source) {
      load ??= createOnDemandLoader({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT });
      messages = await load(r.locale);
    }
    const res = new Response(page.body, page);
    if (d?.header) res.headers.set("cache-control", "private, no-store"); // a variant is never shared
    else if (r.locale !== source && !messages) res.headers.set("cache-control", "no-store"); // never cache the source under the German URL
    for (const c of d?.cookies ?? []) res.headers.append("set-cookie", c);
    const overlay = d?.overlay ?? {};
    if (!messages && !Object.keys(overlay).length) return res;
    const lookup = (key: string) => overlay[key] ?? key.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown> | undefined)?.[k], messages);
    return new HTMLRewriter()
      .on("html", { element: (e) => e.setAttribute("lang", r.locale) })
      .on("[data-glot]", {
        element(e) {
          const v = lookup(e.getAttribute("data-glot") ?? "");
          if (typeof v === "string") e.setInnerContent(v); // text, HTML-escaped by HTMLRewriter
        },
      })
      .transform(res);
  },
};
