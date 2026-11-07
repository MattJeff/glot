// Express / Connect / plain `node:http` middleware: glot Edge for apps that keep everything on their own server.
//   import { glotEdge, fileStore } from "@glot/edge/express";
//   app.use(glotEdge({ source: "en", locales: ["es"], server: "https://api.glotlang.com", project: "site",
//                      key: "pk_…", store: fileStore(".glot-edge") }));
// `/es/x` = this app's own `/x`, fetched over loopback (header `x-glot-edge-origin: 1` → not routed again) and
// translated from the TM. Fail-open: glot down → the page untranslated. `store` keeps the last TM on disk.
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { join } from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";
import { route, type RouteOptions } from "./route.ts";
import { createEdge, type EdgeOptions, type Store } from "./edge.ts";
import { translate } from "./translate.ts";

export type ExpressEdgeOptions = RouteOptions &
  Omit<EdgeOptions, "source" | "translate"> & {
    /** Where to fetch this app's pages; default `http://127.0.0.1:{the server's port}`. */
    origin?: string;
    translate?: EdgeOptions["translate"];
  };

/** Last known TM/status as files in `dir` (atomic rename). */
export function fileStore(dir: string): Store {
  const file = (k: string) => join(dir, encodeURIComponent(k) + ".json");
  return {
    get: (k) => readFile(file(k), "utf8").catch(() => null),
    async put(k, v) {
      await mkdir(dir, { recursive: true });
      const tmp = `${file(k)}.${process.pid}.tmp`;
      await writeFile(tmp, v);
      await rename(tmp, file(k));
    },
  };
}

export function glotEdge(o: ExpressEdgeOptions) {
  const edge = createEdge({ translate, ...o });
  const send = async (res: ServerResponse, out: Response) => {
    const h: Record<string, string | string[]> = {};
    out.headers.forEach((v, k) => (h[k] = v));
    const cookies = out.headers.getSetCookie();
    if (cookies.length) h["set-cookie"] = cookies;
    res.writeHead(out.status, h);
    res.end(Buffer.from(await out.arrayBuffer()));
  };
  return async (req: IncomingMessage, res: ServerResponse, next: (e?: unknown) => void) => {
    try {
      if (req.headers["x-glot-edge-origin"] === "1") return next();
      const host = req.headers.host ?? "localhost";
      const url = new URL(req.url ?? "/", `http://${host}`);
      const headers = new Headers();
      for (const [k, v] of Object.entries(req.headers)) if (typeof v === "string") headers.set(k, v);
      else if (Array.isArray(v)) headers.set(k, v.join(", "));
      if (url.pathname.startsWith("/__glot/")) {
        // ponytail: reads the raw body (a body parser mounted before us would have consumed it: mount glotEdge first)
        const body = req.method === "POST" ? await new Promise<string>((ok, no) => {
          let b = "";
          req.setEncoding("utf8");
          req.on("data", (c: string) => { if ((b += c).length > 65_536) req.destroy(); }).on("end", () => ok(b)).on("error", no);
        }) : undefined;
        const own = await edge.own(new Request(url, { method: req.method, headers, body }));
        if (own) return void (await send(res, own));
      }
      const r = route(url, headers, o);
      if ("redirect" in r) {
        res.writeHead(302, { location: r.redirect, vary: "Accept-Language, Cookie" }).end();
        return;
      }
      if (r.locale === o.source) return next();
      if (req.method !== "GET" && req.method !== "HEAD") {
        req.url = r.path + url.search; // forms, APIs: the app's own route, untouched
        return next();
      }
      const base = o.origin ?? `http://127.0.0.1:${req.socket.localPort}`;
      const originUrl = new URL(r.path + url.search, base);
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
