// glot Edge runtime (edge.ts): version pinning, fail-open, kill switch, stale-if-error, page cache, discovery.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createEdge, cacheTtl, lookup, type Translator, type Store, type PageCache } from "../src/edge.ts";

// Fake segmenter: every `<p>text</p>` is a segment keyed by its text.
const translate: Translator = (html, tm) => {
  let total = 0, translated = 0;
  const missing: { hash: string; text: string; role: string; discoverable?: boolean }[] = [];
  const out = html.replace(/<p>([^<]*)<\/p>/g, (_, t: string) => {
    total++;
    const v = lookup(tm, t, "p|/pricing");
    if (v !== undefined) return translated++, `<p>${v}</p>`;
    missing.push({ hash: t, text: t, role: "p", discoverable: !t.includes("@") });
    return `<p>${t}</p>`;
  });
  return { html: out, total, translated, missing };
};

type Glot = { status?: () => Response | Promise<Response>; tm?: (since: string | null) => Response | Promise<Response>; locales?: () => Response };
function setup(glot: Glot, extra: Partial<Parameters<typeof createEdge>[0]> = {}) {
  const posted: any[] = [];
  const calls: string[] = [];
  const f = (async (input: string, init?: RequestInit) => {
    const u = new URL(input);
    calls.push(u.pathname + u.search);
    if (u.pathname.endsWith("/status")) return glot.status ? glot.status() : new Response("down", { status: 503 });
    if (u.pathname.endsWith("/tm")) return glot.tm ? glot.tm(u.searchParams.get("since")) : new Response("down", { status: 503 });
    if (u.pathname.endsWith("/missing")) return posted.push(JSON.parse(String(init?.body))), new Response("{}");
    if (u.pathname === "/cdn/site/locales.json" && glot.locales) return glot.locales();
    throw new Error("unexpected " + input);
  }) as unknown as typeof fetch;
  const edge = createEdge({ server: "https://glot", project: "site", key: "pk_x", source: "en", translate, fetch: f, timeout: 50, ...extra });
  return { edge, posted, calls };
}
const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { "content-type": "application/json" } });
const page = (body = "<html><head></head><body><p>Hello</p><p>Save</p></body></html>", h: Record<string, string> = {}) => async () =>
  new Response(body, { headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": "default-src 'self'", ...h } });
const req = (path = "/es/pricing", h: Record<string, string> = {}) => new Request(`https://site.test${path}`, { headers: { accept: "text/html", ...h } });
const run = (e: ReturnType<typeof setup>["edge"], r = req(), origin = page()) => {
  const bg: Promise<unknown>[] = [];
  return e(r, "es", new URL("https://origin.test/pricing?q=secret"), origin, (p) => bg.push(p)).then(async (res) => ({ res, body: await res.text(), bg: await Promise.all(bg) }));
};

test("translates with one TM snapshot, keeps security headers, Server-Timing", async () => {
  const { edge } = setup({ status: () => json({ enabled: true, off: [], versions: { es: 3 } }), tm: () => json({ v: 3, t: { Hello: "Hola", Save: "Guardar" } }) });
  const { res, body } = await run(edge);
  assert.match(body, /<p>Hola<\/p><p>Guardar<\/p>/);
  assert.equal(res.headers.get("x-glot-edge"), "translated");
  assert.equal(res.headers.get("content-language"), "es");
  assert.equal(res.headers.get("x-glot-version"), "es@3");
  assert.equal(res.headers.get("x-glot-coverage"), "2/2");
  assert.equal(res.headers.get("content-security-policy"), "default-src 'self'");
  assert.match(res.headers.get("server-timing")!, /^glot;dur=[\d.]+;desc="translated miss", origin;dur=[\d.]+$/);
});

test("fail-open: glot down → origin HTML untranslated, not cached, never 5xx", async () => {
  const { edge } = setup({});
  const { res, body } = await run(edge);
  assert.equal(res.status, 200);
  assert.match(body, /<p>Hello<\/p>/);
  assert.equal(res.headers.get("x-glot-edge"), "fail-open");
  assert.equal(res.headers.get("cache-control"), "no-store");
});

test("fail-open: glot hangs → served within the timeout budget", async () => {
  const never = () => new Promise<Response>(() => {});
  const { edge } = setup({ status: never, tm: never });
  const t = Date.now();
  const bg: Promise<unknown>[] = [];
  const res = await edge(req(), "es", new URL("https://o/pricing"), page(), (p) => bg.push(p)); // background work never awaited
  assert.ok(Date.now() - t < 500);
  assert.equal(res.headers.get("x-glot-edge"), "fail-open");
});

test("fail-open: translator throws → origin page", async () => {
  const { edge } = setup({ status: () => json({ enabled: true, off: [], versions: { es: 1 } }), tm: () => json({ v: 1, t: {} }) }, {
    translate: () => {
      throw new Error("boom");
    },
  });
  const { res, body } = await run(edge);
  assert.equal(res.headers.get("x-glot-edge"), "fail-open");
  assert.match(body, /Hello/);
});

test("kill switch: project off or host off → passthrough", async () => {
  for (const st of [{ enabled: false, off: [], versions: {} }, { enabled: true, off: ["site.test"], versions: { es: 1 } }]) {
    const { edge, calls } = setup({ status: () => json(st), tm: () => json({ v: 1, t: { Hello: "Hola" } }) });
    const { res, body } = await run(edge);
    assert.equal(res.headers.get("x-glot-edge"), "off");
    assert.match(body, /<p>Hello<\/p>/);
    assert.ok(!calls.some((c) => c.includes("/tm")), "no TM fetch when off");
  }
});

test("kill switch propagates on the next status revalidation (< statusTtl)", async () => {
  let enabled = true, t = 0;
  const { edge } = setup(
    { status: () => json({ enabled, off: [], versions: { es: 1 } }), tm: () => json({ v: 1, t: { Hello: "Hola" } }) },
    { now: () => t, statusTtl: 20_000 },
  );
  assert.match((await run(edge)).body, /Hola/);
  enabled = false;
  t = 21_000;
  await run(edge); // stale: served with the old status, revalidated in the background
  const { res } = await run(edge);
  assert.equal(res.headers.get("x-glot-edge"), "off");
});

test("version bump: the old snapshot is served whole while the new one loads, then the new one, never a mix", async () => {
  let v = 1, t = 0;
  const tms: Record<number, Record<string, string>> = { 1: { Hello: "Hola v1", Save: "Guardar v1" }, 2: { Hello: "Hola v2", Save: "Guardar v2" } };
  const { edge, calls } = setup({ status: () => json({ enabled: true, off: [], versions: { es: v } }), tm: (since) => json({ v: v, t: since ? { Hello: tms[v].Hello, Save: tms[v].Save } : tms[v] }) }, { now: () => t });
  assert.match((await run(edge)).body, /Hola v1.*Guardar v1/);
  v = 2;
  t = 30_000;
  const mid = (await run(edge)).body;
  assert.ok(/Hola v1.*Guardar v1/.test(mid) || /Hola v2.*Guardar v2/.test(mid), mid);
  const { body, res } = await run(edge);
  assert.match(body, /Hola v2.*Guardar v2/);
  assert.equal(res.headers.get("x-glot-version"), "es@2");
  assert.equal(calls.filter((c) => c.endsWith("/tm")).length, 2, "one TM fetch per version");
});

test("stale-if-error: glot goes down after a first load → last TM still served", async () => {
  let up = true, t = 0;
  const { edge } = setup(
    { status: () => (up ? json({ enabled: true, off: [], versions: { es: 1 } }) : new Response("", { status: 502 })), tm: () => (up ? json({ v: 1, t: { Hello: "Hola" } }) : new Response("", { status: 502 })) },
    { now: () => t },
  );
  await run(edge);
  up = false;
  t = 60_000;
  for (let i = 0; i < 3; i++) assert.match((await run(edge)).body, /Hola/);
});

test("persistent store: a fresh isolate with glot down serves the last TM from disk", async () => {
  const mem = new Map<string, string>();
  const store: Store = { get: async (k) => mem.get(k) ?? null, put: async (k, v) => void mem.set(k, v) };
  const a = setup({ status: () => json({ enabled: true, off: [], versions: { es: 4 } }), tm: () => json({ v: 4, t: { Hello: "Hola" } }) }, { store });
  await run(a.edge);
  await new Promise((r) => setTimeout(r, 10));
  const b = setup({}, { store }); // glot down
  const { res, body } = await run(b.edge);
  assert.match(body, /Hola/);
  assert.equal(res.headers.get("x-glot-version"), "es@4");
});

test("page cache: key has the version (bump = purge); private/cookie pages never cached", async () => {
  const cache = new Map<string, Response>();
  const pageCache: PageCache = { match: async (k) => cache.get(k)?.clone(), put: async (k, r) => void cache.set(k, r) };
  let v = 1;
  const { edge } = setup({ status: () => json({ enabled: true, off: [], versions: { es: v } }), tm: () => json({ v: v, t: { Hello: `Hola ${v}` } }) }, { pageCache, statusTtl: 0 });
  let origin = 0;
  const o = async () => (origin++, page(undefined, { "cache-control": "public, max-age=60" })());
  await run(edge, req(), o);
  const hit = await run(edge, req(), o);
  assert.equal(origin, 1);
  assert.equal(hit.res.headers.get("x-glot-edge"), "translated; cache=hit");
  assert.equal([...cache.keys()][0], "https://glot-edge.invalid/site.test/pricing?q=secret&__glot=es.1");
  v = 2;
  await run(edge, req(), o); // new status → old snapshot served once…
  await run(edge, req(), o);
  assert.ok([...cache.keys()].some((k) => k.endsWith("__glot=es.2")));
  const before = cache.size;
  await run(edge, req("/es/pricing", { cookie: "sessionid=abc" }), o);
  assert.equal(cache.size, before);
  assert.equal(cacheTtl(new Headers({ "cache-control": "private, max-age=60" }), 30), 0);
  assert.equal(cacheTtl(new Headers({ "cache-control": "public, s-maxage=5, max-age=60" }), 30), 5);
  assert.equal(cacheTtl(new Headers({ "set-cookie": "a=b" }), 30), 0);
});

test("discovery: path only, deduped, PII segments never sent, nothing for logged-in requests", async () => {
  const st = () => json({ enabled: true, off: [], versions: { es: 1 } });
  const { edge, posted } = setup({ status: st, tm: () => json({ v: 1, t: {} }) });
  const o = page("<html><head></head><p>New copy</p><p>me@x.io</p></html>");
  await run(edge, req(), o);
  await run(edge, req(), o);
  assert.deepEqual(posted, [{ key: "pk_x", locale: "es", auth: false, segments: [{ hash: "New copy", text: "New copy", role: "p", url: "/pricing" }], pages: [] }]);
  const b = setup({ status: st, tm: () => json({ v: 1, t: {} }) });
  await run(b.edge, req("/es/pricing", { cookie: "next-auth.session-token=x" }), o);
  await run(b.edge, req("/es/pricing", { authorization: "Bearer x" }), o);
  assert.deepEqual(b.posted, []);
});

test("non-HTML, non-200 and POST pass through untouched", async () => {
  const { edge } = setup({ status: () => json({ enabled: true, off: [], versions: { es: 1 } }), tm: () => json({ v: 1, t: { Hello: "Hola" } }) });
  const img = await run(edge, req(), async () => new Response("<p>Hello</p>", { headers: { "content-type": "image/svg+xml" } }));
  assert.equal(img.body, "<p>Hello</p>");
  const nf = await run(edge, req(), async () => new Response("<p>Hello</p>", { status: 404, headers: { "content-type": "text/html" } }));
  assert.equal(nf.res.status, 404);
  assert.equal(nf.body, "<p>Hello</p>");
  const post = await edge(new Request("https://site.test/es/form", { method: "POST", body: "a=1" }), "es", new URL("https://o/form"), page());
  assert.equal(post.headers.get("x-glot-edge"), "passthrough");
});

test("plan quota: usage flushed with the key, a 402 serves the origin untranslated", async () => {
  let t = 0;
  const usage: any[] = [];
  const f = (async (input: string, init?: RequestInit) => {
    const u = new URL(input);
    if (u.pathname.endsWith("/status")) return json({ enabled: true, off: [], versions: { es: 1 } });
    if (u.pathname.endsWith("/tm")) return json({ v: 1, t: { Hello: "Hola" } });
    if (u.pathname.endsWith("/usage")) return usage.push(JSON.parse(String(init?.body))), new Response("", { status: 402 });
    return new Response("{}");
  }) as unknown as typeof fetch;
  const edge = createEdge({ server: "https://glot", project: "site", key: "pk_x", source: "en", translate, fetch: f, now: () => t });
  assert.match((await run(edge)).body, /Hola/);
  t = 31_000;
  assert.match((await run(edge)).body, /Hola/); // this one flushes: 2 pages
  assert.deepEqual(usage, [{ key: "pk_x", pages: 2 }]);
  const { res, body } = await run(edge);
  assert.equal(res.headers.get("x-glot-edge"), "quota");
  assert.match(body, /<p>Hello<\/p>/);
});

test("hreflang lists only READY locales (/cdn/{project}/locales.json) + the page's own; last known list if it fails", async () => {
  let seen: readonly string[] = [];
  let manifest: () => Response = () => json({ source: "en", locales: [{ code: "en", ready: true }, { code: "de", ready: true }, { code: "fr", ready: false }] });
  const { edge } = setup(
    { status: () => json({ enabled: true, off: [], versions: { es: 1 } }), tm: () => json({ v: 1, t: {} }), locales: () => manifest() },
    { locales: ["es", "fr", "de"], statusTtl: 0, translate: (html, tm, c) => ((seen = c.locales), translate(html, tm, c)) },
  );
  await run(edge);
  assert.deepEqual(seen, ["es", "de"]);
  manifest = () => new Response("down", { status: 503 });
  await run(edge);
  assert.deepEqual(seen, ["es", "de"]);
});

test("own routes: tm.json `since` (204 until newer) + project skip selectors; /__glot/missing forwarded with key and auth", async () => {
  const { edge, posted } = setup({ status: () => json({ enabled: true, off: [], versions: { es: 4 }, selectors: { include: [], exclude: [".chat"] } }), tm: () => json({ v: 4, t: { a: "b" } }) });
  const tm = await (await edge.own(new Request("https://site.test/__glot/tm.json?locale=es")))!.json();
  assert.deepEqual([tm.v, tm.skip], [4, [".chat"]]);
  assert.equal((await edge.own(new Request("https://site.test/__glot/tm.json?locale=es&since=4")))!.status, 204);
  assert.equal((await edge.own(new Request("https://site.test/__glot/tm.json?locale=es&since=3")))!.status, 200);
  const body = JSON.stringify({ locale: "es", segments: [
    { hash: "h1", text: "Play", role: "button", url: "/es/cliping?email=a@b.c#x" },
    { hash: "h2", text: "x".repeat(3000), role: "p", url: "/a" }, // too long
    { text: "no hash", url: "/a" },
    { hash: "h3", text: "Bad url", url: "https://evil.test/" },
  ] });
  const post = (cookie = "") => new Request("https://site.test/__glot/missing", { method: "POST", body, headers: cookie ? { cookie } : {} });
  assert.equal((await edge.own(post()))!.status, 204);
  assert.equal((await edge.own(post("theme=dark; next-auth.session-token=abc")))!.status, 204);
  assert.deepEqual(posted, [
    { key: "pk_x", locale: "es", auth: false, segments: [{ hash: "h1", text: "Play", role: "button", url: "/cliping" }] },
    { key: "pk_x", locale: "es", auth: true, segments: [{ hash: "h1", text: "Play", role: "button", url: "/cliping" }] },
  ]);
  // no key: nothing forwarded
  const nokey = setup({}, { key: undefined });
  await nokey.edge.own(post());
  assert.equal(nokey.posted.length, 0);
});

test("signed-in requests: auth.pages server = translated, private, uncached; patcher/off = origin untouched, private", async () => {
  const cache = new Map<string, Response>();
  const pageCache: PageCache = { match: async (k) => cache.get(k)?.clone(), put: async (k, r) => void cache.set(k, r) };
  for (const pages of [undefined, "server", "patcher", "off"] as const) {
    const { edge, posted } = setup({ status: () => json({ enabled: true, off: [], versions: { es: 1 }, auth: pages && { pages } }), tm: () => json({ v: 1, t: { Hello: "Hola" } }) }, { pageCache });
    const o = page(undefined, { "cache-control": "public, max-age=60", "set-cookie": "app_session=rotated; HttpOnly" });
    const { res, body } = await run(edge, req("/es/pricing", { cookie: "app_session=abc" }), o);
    const server = pages === undefined || pages === "server";
    assert.equal(res.headers.get("x-glot-edge"), server ? "translated" : "auth", String(pages));
    assert.match(body, server ? /<p>Hola<\/p>/ : /<p>Hello<\/p>/);
    assert.equal(res.headers.get("cache-control"), "private, no-cache");
    assert.match(res.headers.get("vary")!, /Cookie/);
    assert.equal(res.headers.get("set-cookie"), "app_session=rotated; HttpOnly"); // the app's session, as is
    assert.equal(cache.size, 0);
    assert.deepEqual(posted, []);
  }
});
