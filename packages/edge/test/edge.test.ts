// Routing rules + the Worker flow (fake origin, fake glot CDN, minimal HTMLRewriter stand-in).
import { test } from "node:test";
import assert from "node:assert/strict";
import { route } from "../src/route.ts";

const o = { source: "en", locales: ["de", "it"] };
const h = (x: Record<string, string>) => new Headers(x);
const html = { accept: "text/html,*/*" };

test("route: prefix, redirect, source", () => {
  assert.deepEqual(route(new URL("https://s/de/pricing"), h({}), o), { locale: "de", path: "/pricing" });
  assert.deepEqual(route(new URL("https://s/faq"), h({}), o), { locale: "en", path: "/faq" }); // not a locale
  assert.deepEqual(route(new URL("https://s/pricing?a=1"), h({ ...html, "accept-language": "de-AT,en;q=0.5" }), o), { redirect: "/de/pricing?a=1" });
  assert.deepEqual(route(new URL("https://s/x"), h({ ...html, "accept-language": "ja" }), o), { locale: "en", path: "/x" }); // not enabled
  assert.deepEqual(route(new URL("https://s/x"), h({ ...html, "accept-language": "de", cookie: "glot_locale=en" }), o), { locale: "en", path: "/x" });
  assert.deepEqual(route(new URL("https://s/x.png"), h({ "accept-language": "de" }), o), { locale: "en", path: "/x.png" }); // not HTML
});

test("worker: source while translating, data-glot filled once ready", async () => {
  let ready = false;
  const origin = '<html lang="en"><h1 data-glot="home.title">Welcome</h1><p>static</p></html>';
  globalThis.fetch = (async (input: Request | string) => {
    const u = String(typeof input === "string" ? input : input.url);
    if (u.startsWith("https://glot/cdn/site/de.json")) {
      return ready
        ? new Response('{"home":{"title":"Willkommen <3"}}', { status: 200, headers: { etag: '"1"' } })
        : new Response("{}", { status: 202, headers: { "retry-after": "0" } });
    }
    assert.equal(new URL(u).pathname, "/pricing");
    return new Response(origin, { headers: { "content-type": "text/html" } });
  }) as typeof fetch;
  // Stand-in for Cloudflare's HTMLRewriter: enough for `html` + `[data-glot]` on flat elements.
  (globalThis as any).HTMLRewriter = class {
    h: [string, any][] = [];
    on(sel: string, handler: any) { this.h.push([sel, handler]); return this; }
    transform(res: Response) {
      const out = res.text().then((s) => {
        for (const [sel, handler] of this.h) {
          const re = sel === "html" ? /<html([^>]*)>/g : /<(\w+) data-glot="([^"]+)">([^<]*)<\/\1>/g;
          s = s.replace(re, (...m: string[]) => {
            let attr = "", inner = m[3];
            handler.element({
              getAttribute: () => m[2],
              setAttribute: (k: string, v: string) => (attr = ` ${k}="${v}"`),
              setInnerContent: (v: string) => (inner = v.replace(/</g, "&lt;")),
            });
            return sel === "html" ? `<html${attr}>` : `<${m[1]} data-glot="${m[2]}">${inner}</${m[1]}>`;
          });
        }
        return s;
      });
      return new Response(new ReadableStream({ async start(c) { c.enqueue(new TextEncoder().encode(await out)); c.close(); } }));
    }
  };
  const { default: worker } = await import("../src/worker.ts");
  const env = { GLOT_SERVER: "https://glot", GLOT_PROJECT: "site", LOCALES: "de" };
  const get = () => worker.fetch(new Request("https://s/de/pricing", { headers: html }), env);

  const first = await get();
  assert.equal(first.headers.get("cache-control"), "no-store");
  assert.match(await first.text(), /Welcome/);

  ready = true;
  const body = await (await get()).text();
  assert.match(body, /<html lang="de">/);
  assert.match(body, /<h1 data-glot="home.title">Willkommen &lt;3<\/h1>/);
  assert.match(body, /static/);
});
