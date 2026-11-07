// Application paths: shared vectors with the Rust proxy (fixtures/app-paths.json) + the runtimes' wiring.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { classify, decide, normalize, type AppAction } from "../src/app-paths.ts";
import { createEdge } from "../src/edge.ts";
import { glotEdge } from "../src/express.ts";

const F = JSON.parse(readFileSync(new URL("./fixtures/app-paths.json", import.meta.url), "utf8"));

test("shared vectors: normalize, classify, decide", () => {
  for (const [i, o] of F.normalize) assert.equal(normalize(i), o, i);
  for (const g of F.classify) for (const [p, want] of g.cases) assert.equal(classify(g.rules, g.action as AppAction, p) ?? null, want, p);
  for (const c of F.decide) assert.deepEqual(decide(c.in) ?? { kind: "none" }, c.out, c.name);
});

const APP = { rules: ["/login", "proxy /api/"], action: "redirect", locale: "query:lang" };
const glotFetch = (async (input: string) => {
  const u = new URL(String(input));
  if (u.pathname.endsWith("/status")) return new Response(JSON.stringify({ enabled: true, off: [], versions: { es: 1 }, app: APP }));
  if (u.pathname.endsWith("/es/tm")) return new Response(JSON.stringify({ v: 1, locale: "es", t: { Hello: "Hola" } }));
  return new Response("{}", { status: 404 });
}) as unknown as typeof fetch;

test("runtime: rules from the status, redirect to the source URL with ?lang, proxy, normal pages", async () => {
  const edge = createEdge({ server: "https://glot", project: "site", source: "en", translate: (html) => ({ html, total: 0, translated: 0, missing: [] }), fetch: glotFetch, timeout: 200 });
  const r = await edge.app(new Request("https://site.test/es/login?next=%2Fx", { headers: { cookie: "a=1" } }), "es", "/login");
  assert.ok(r instanceof Response);
  assert.equal(r.status, 302);
  assert.equal(r.headers.get("location"), "https://site.test/login?next=%2Fx&lang=es");
  assert.equal(r.headers.get("cache-control"), "no-store");
  assert.match(r.headers.get("set-cookie")!, /^glot_app_hop=\/login;/);
  const post = await edge.app(new Request("https://site.test/es/login", { method: "POST", body: "x" }), "es", "/login");
  assert.equal((post as Response).status, 307);
  assert.equal(await edge.app(new Request("https://site.test/es/api/me"), "es", "/api/me"), "proxy");
  assert.equal(await edge.app(new Request("https://site.test/es/pricing"), "es", "/pricing"), undefined);
  assert.equal(await edge.isApp("/login"), true);
  assert.equal(await edge.isApp("/loginx"), false);
});

const listen = (s: Server) => new Promise<number>((ok) => s.listen(0, "127.0.0.1", () => ok((s.address() as { port: number }).port)));

test("express: app redirect, proxy untouched (Set-Cookie kept, never translated), no locale redirect for app paths", async () => {
  const glot = createServer(async (req, res) => {
    const r = await glotFetch(`http://g${req.url}`);
    res.statusCode = r.status;
    res.end(await r.text());
  });
  const gp = await listen(glot);
  const mw = glotEdge({ source: "en", locales: ["es"], server: `http://127.0.0.1:${gp}`, project: "site", timeout: 500 });
  const up = createServer((req, res) =>
    mw(req, res, () => {
      res.setHeader("content-type", "text/html");
      res.setHeader("set-cookie", "session=abc; HttpOnly");
      res.end(`<p>Hello</p> ${req.method} ${req.url}`);
    }),
  );
  const port = await listen(up);
  const base = `http://127.0.0.1:${port}`;
  try {
    const r = await fetch(`${base}/es/login?lang=fr`, { redirect: "manual" });
    assert.equal(r.status, 302);
    assert.equal(r.headers.get("location"), `${base}/login?lang=es`);
    const p = await fetch(`${base}/es/api/me`, { method: "POST", body: "x" });
    assert.equal(await p.text(), "<p>Hello</p> POST /api/me");
    assert.equal(p.headers.get("set-cookie"), "session=abc; HttpOnly");
    const g = await fetch(`${base}/es/api/me`);
    assert.equal(await g.text(), "<p>Hello</p> GET /api/me", "never translated");
    // A Spanish visitor on /login is not sent to /es/login (which would send them back: loop).
    const s = await fetch(`${base}/login?lang=es`, { redirect: "manual", headers: { accept: "text/html", "accept-language": "es" } });
    assert.equal(s.status, 200);
    assert.equal(await s.text(), "<p>Hello</p> GET /login?lang=es");
    const t = await fetch(`${base}/es/pricing`);
    assert.equal(t.headers.get("x-glot-edge"), "source", "a normal page goes through Edge");
  } finally {
    up.close();
    glot.close();
  }
});

test("next proxy: app redirect, proxy = the app's own route (rewrite), no locale redirect for app paths", async () => {
  const { glotProxy } = await import("../src/next-proxy.ts");
  const { NextRequest } = await import("next/server.js");
  const proxy = glotProxy({ source: "en", locales: ["es"], edge: { server: "https://glot", project: "site", fetch: glotFetch } });
  const r = await proxy(new NextRequest("https://s.test/es/login", { headers: { accept: "text/html" } }));
  assert.equal(r.status, 302);
  assert.equal(r.headers.get("location"), "https://s.test/login?lang=es");
  const p = await proxy(new NextRequest("https://s.test/es/api/me", { method: "POST", body: "x" }));
  assert.equal(p.headers.get("x-middleware-rewrite"), "https://s.test/api/me");
  const s = await proxy(new NextRequest("https://s.test/login", { headers: { accept: "text/html", "accept-language": "es" } }));
  assert.equal(s.headers.get("location"), null);
});

test("worker: app redirect and proxy (origin response untouched)", async () => {
  const saved = globalThis.fetch;
  let hit = "";
  globalThis.fetch = (async (input: Request | string) => {
    const u = String(typeof input === "string" ? input : input.url);
    if (u.startsWith("https://glot/")) return glotFetch(u);
    hit = u;
    return new Response("<p>Hello</p>", { headers: { "content-type": "text/html", "set-cookie": "session=abc" } });
  }) as typeof fetch;
  try {
    const { default: worker } = await import("../src/worker.ts");
    const env = { GLOT_SERVER: "https://glot", GLOT_PROJECT: "site", LOCALES: "es" };
    const r = await worker.fetch(new Request("https://w.test/es/login?x=1"), env);
    assert.equal(r.status, 302);
    assert.equal(r.headers.get("location"), "https://w.test/login?x=1&lang=es");
    const p = await worker.fetch(new Request("https://w.test/es/api/me?q=1"), env);
    assert.equal(hit, "https://w.test/api/me?q=1");
    assert.equal(p.headers.get("set-cookie"), "session=abc");
    assert.equal(await p.text(), "<p>Hello</p>");
  } finally {
    globalThis.fetch = saved;
  }
});
