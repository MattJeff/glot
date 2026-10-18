// Cloudflare Worker in front of a static / marketing site (route it on your zone; `fetch(req)` reaches the origin).
// `/de/pricing` = the origin's `/pricing` with every `data-glot="key"` element filled from glot's `de` catalog,
// asked on demand: while glot translates (first visit), the source page is served uncached.
//   wrangler.toml: main = "node_modules/@glot/edge/dist/worker.js"
//   [vars] GLOT_SERVER = "https://glot.example.com"  GLOT_PROJECT = "site"  LOCALES = "de,it,ja"  SOURCE_LOCALE = "en"
// ponytail: plain strings only (no ICU formatting) and links inside the page are not rewritten to `/de/…`.
import { createOnDemandLoader } from "@glot/runtime/ondemand";
import { route } from "./route.ts";

export interface Env {
  GLOT_SERVER: string;
  GLOT_PROJECT: string;
  /** Comma-separated locales served under `/{locale}/` (glot's on-demand whitelist). */
  LOCALES: string;
  SOURCE_LOCALE?: string;
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

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const source = env.SOURCE_LOCALE ?? "en";
    const r = route(url, req.headers, { source, locales: env.LOCALES.split(",").map((l) => l.trim()) });
    if ("redirect" in r) return new Response(null, { status: 302, headers: { location: r.redirect, vary: "Accept-Language, Cookie" } });
    if (r.locale === source) return fetch(req);

    url.pathname = r.path;
    const page = await fetch(new Request(url, req));
    if (!page.headers.get("content-type")?.includes("text/html")) return page;
    load ??= createOnDemandLoader({ server: env.GLOT_SERVER, project: env.GLOT_PROJECT });
    const messages = await load(r.locale);
    if (!messages) {
      const res = new Response(page.body, page);
      res.headers.set("cache-control", "no-store"); // never cache the source under the German URL
      return res;
    }
    const lookup = (key: string) => key.split(".").reduce<unknown>((o, k) => (o as Record<string, unknown> | undefined)?.[k], messages);
    return new HTMLRewriter()
      .on("html", { element: (e) => e.setAttribute("lang", r.locale) })
      .on("[data-glot]", {
        element(e) {
          const v = lookup(e.getAttribute("data-glot") ?? "");
          if (typeof v === "string") e.setInnerContent(v); // text, HTML-escaped by HTMLRewriter
        },
      })
      .transform(page);
  },
};
