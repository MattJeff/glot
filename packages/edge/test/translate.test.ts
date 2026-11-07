// JS page translator (real segmenter, TM wire format) + the Express middleware end to end over real HTTP.
import { test } from "node:test";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { hash16 } from "../src/patcher.ts";
import { translate, seo } from "../src/translate.ts";
import { glotEdge } from "../src/express.ts";
import type { Tm } from "../src/edge.ts";

const PAGE = `<!doctype html><html lang="en"><head><title>Simple pricing</title>
<link rel="canonical" href="https://site.test/pricing"><link rel="alternate" hreflang="en" href="https://site.test/pricing"></head>
<body><h1>Simple pricing</h1><button>Save</button><td>Save</td>
<a href="/signup">Create an account</a> <a href="/es/x">ES</a> <a href="/logo.png">Logo</a> <a href="https://other.test/">Other</a>
<p>Brand new paragraph</p><p>Write to me@private.io</p>
<script id="__NEXT_DATA__" type="application/json">{"props":{"title":"Simple pricing","id":"Simple pricing"}}</script></body></html>`;
const h = (t: string) => hash16(`en\u0000${t}`);
const TM: Tm = {
  v: 7,
  locale: "es",
  t: {
    [h("Simple pricing")]: "Precios sencillos",
    [h("Save")]: { "button|/pricing": "Guardar", "td|/pricing": "Ahorra" },
    [h("Create an account")]: "Crear una cuenta",
  },
};
const ctx = { url: new URL("https://site.test/es/pricing"), path: "/pricing", locale: "es", source: "en", locales: ["es", "fr"], skip: [] };

test("translate: segments by context, payload, SEO, version + patcher, PII never reported", async () => {
  const r = translate(PAGE, TM, ctx);
  const out = r.html;
  for (const want of [
    "<title>Precios sencillos</title>", "<h1>Precios sencillos</h1>", "<button>Guardar</button>", "<td>Ahorra</td>",
    '<a href="/es/signup">Crear una cuenta</a>', '<a href="/es/x">', '<a href="/logo.png">', '<a href="https://other.test/">',
    '<html lang="es" dir="ltr">', '<link rel="canonical" href="https://site.test/es/pricing">',
    '<link rel="alternate" hreflang="en" href="https://site.test/pricing">', '<link rel="alternate" hreflang="fr" href="https://site.test/fr/pricing">',
    '<link rel="alternate" hreflang="x-default" href="https://site.test/pricing">', '<meta name="glot-tm" content="es@7">',
    'src="/__glot/patcher.js" data-tm="/__glot/tm.json?locale=es"', '"title":"Precios sencillos","id":"Simple pricing"',
  ]) assert.ok(out.includes(want), `missing ${want}\n${out}`);
  assert.equal(out.match(/rel="canonical"/g)?.length, 1);
  assert.equal(out.match(/hreflang="en"/g)?.length, 1);
  assert.equal(r.translated, 5);
  const missing = r.missing.map((m) => m.text);
  assert.ok(missing.includes("Brand new paragraph"), String(missing));
  assert.ok(!missing.some((m) => m.includes("@") || m.includes("Write to")), "personal data is never reported");
  assert.ok(r.seen!.length >= 7);
});

// Same cases through glot_core::edge::seo::rewrite (scratch oracle, see seo-expected.json): byte for byte.
test("seo = the Rust SEO rewrite on every case (canonical, hreflang, og, JSON-LD, links, lol_html serialization)", () => {
  const cases = JSON.parse(readFileSync(new URL("./fixtures/seo-cases.json", import.meta.url), "utf8")) as
    { name: string; source: string; locale: string; path: string; bases: [string, string][]; tr: Record<string, string>; html: string }[];
  const want = JSON.parse(readFileSync(new URL("./fixtures/seo-expected.json", import.meta.url), "utf8")) as Record<string, string>;
  for (const c of cases) assert.equal(seo(c.html, { ...c, tr: (t) => c.tr[t] }), want[c.name], c.name);
});

test("translate: hreflang lists the locales it is given (edge.ts passes the ready ones), JSON-LD through the TM", () => {
  const page = `<html><head><script type="application/ld+json">{"name":"Simple  pricing","inLanguage":"en"}</script></head><body></body></html>`;
  const tm: Tm = { v: 1, locale: "es", t: { [h("Simple pricing")]: { "text|/pricing": "Precios sencillos" } } };
  const out = translate(page, tm, { ...ctx, locales: ["es"] }).html;
  assert.match(out, /"name":"Precios sencillos","inLanguage":"es"/);
  assert.doesNotMatch(out, /hreflang="fr"/);
  assert.doesNotMatch(out, /data-missing/);
  assert.match(translate(page, tm, { ...ctx, discover: true }).html, /data-missing="\/__glot\/missing"/);
});

const listen = (s: Server) => new Promise<number>((ok) => s.listen(0, "127.0.0.1", () => ok((s.address() as { port: number }).port)));

test("express: /es/pricing translated over loopback, POST untouched, /__glot/tm.json, fail-open when glot is down", async () => {
  let reports = 0;
  const glot = createServer((req, res) => {
    if (req.url?.endsWith("/status")) return res.end(JSON.stringify({ enabled: true, off: [], versions: { es: 7 } }));
    if (req.url?.endsWith("/es/tm")) return res.end(JSON.stringify(TM));
    if (req.url?.endsWith("/missing")) return (reports++, res.end("{}"));
    res.statusCode = 404;
    res.end();
  });
  const glotPort = await listen(glot);
  const mw = (server: string) => glotEdge({ source: "en", locales: ["es"], server, project: "site", key: "pk_t", timeout: 500 });
  const app = (m: ReturnType<typeof glotEdge>) =>
    createServer((req, res) =>
      m(req, res, () => {
        if (req.method === "POST") return res.end(`posted to ${req.url}`);
        res.setHeader("content-type", "text/html; charset=utf-8");
        res.setHeader("content-security-policy", "script-src 'self'");
        res.end(req.url === "/pricing" ? PAGE : "<p>other</p>");
      }),
    );
  const up = app(mw(`http://127.0.0.1:${glotPort}`));
  const port = await listen(up);
  try {
    const r = await fetch(`http://127.0.0.1:${port}/es/pricing`, { headers: { accept: "text/html" } });
    const body = await r.text();
    assert.equal(r.headers.get("x-glot-edge"), "translated");
    assert.equal(r.headers.get("content-security-policy"), "script-src 'self'");
    assert.equal(r.headers.get("content-language"), "es");
    assert.match(r.headers.get("server-timing")!, /glot;dur=/);
    assert.match(body, /<h1>Precios sencillos<\/h1>/);
    const src = await (await fetch(`http://127.0.0.1:${port}/pricing`)).text();
    assert.equal(src, PAGE, "source pages untouched");
    assert.equal(await (await fetch(`http://127.0.0.1:${port}/es/form`, { method: "POST", body: "a" })).text(), "posted to /form");
    const tm = await (await fetch(`http://127.0.0.1:${port}/__glot/tm.json?locale=es`)).json();
    assert.equal(tm.v, 7);
    assert.match(body, /data-missing="\/__glot\/missing"/);
    const before = reports;
    const m = await fetch(`http://127.0.0.1:${port}/__glot/missing`, { method: "POST", body: JSON.stringify({ locale: "es", segments: [{ hash: "h", text: "Play", role: "button", url: "/es/x" }] }) });
    assert.equal(m.status, 204);
    assert.equal(reports, before + 1);

    // glot unreachable from the start: the origin page, 200, quickly.
    const down = app(mw("http://127.0.0.1:9"));
    const p2 = await listen(down);
    const t = Date.now();
    const f = await fetch(`http://127.0.0.1:${p2}/es/pricing`, { headers: { accept: "text/html" } });
    assert.equal(f.status, 200);
    assert.equal(f.headers.get("x-glot-edge"), "fail-open");
    assert.match(await f.text(), /<h1>Simple pricing<\/h1>/);
    assert.ok(Date.now() - t < 1500);
    down.close();
  } finally {
    up.close();
    glot.close();
  }
});

test("next proxy: /es/pricing = the app's /pricing fetched with x-glot-edge-origin, translated; that fetch passes through", async () => {
  const { glotProxy } = await import("../src/next-proxy.ts");
  const { NextRequest } = await import("next/server.js");
  const glot = (async (input: string) => {
    const u = String(input);
    if (u.endsWith("/status")) return new Response(JSON.stringify({ enabled: true, off: [], versions: { es: 7 } }));
    if (u.endsWith("/es/tm")) return new Response(JSON.stringify(TM));
    return new Response("{}");
  }) as unknown as typeof fetch;
  let seen: Headers | undefined, path = "";
  const saved = globalThis.fetch;
  globalThis.fetch = (async (input: URL | string, init?: RequestInit) => {
    seen = new Headers(init?.headers);
    path = new URL(String(input)).pathname;
    return new Response(PAGE, { headers: { "content-type": "text/html" } });
  }) as typeof fetch;
  try {
    const proxy = glotProxy({ source: "en", locales: ["es"], edge: { server: "https://glot", project: "site", fetch: glot } });
    const res = await proxy(new NextRequest("https://s.test/es/pricing", { headers: { accept: "text/html" } }));
    assert.equal(path, "/pricing");
    assert.equal(seen?.get("x-glot-edge-origin"), "1");
    assert.equal(seen?.get("x-glot-locale"), "es");
    assert.equal(res.headers.get("x-glot-edge"), "translated");
    assert.match(await res.text(), /<h1>Precios sencillos<\/h1>/);
    const inner = await proxy(new NextRequest("https://s.test/pricing", { headers: { "x-glot-edge-origin": "1", "accept-language": "es", accept: "text/html" } }));
    assert.equal(inner.headers.get("x-middleware-next"), "1"); // not redirected, not translated again
  } finally {
    globalThis.fetch = saved;
  }
});
