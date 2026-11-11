// Worker bundle localization (subdirectory mode): the page's language from Referer, the rewrite from glot,
// cache per (URL, locale, version), never `immutable`, 304 on the same ETag, fail-open.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createEdge, type PageCache } from "../src/edge.ts";
import { refererLocale } from "../src/worker.ts";

const SRC = 'jsx("p",{children:"Cancel anytime"})';
const OUT = 'jsx("p",{children:"Cancela cuando quieras"})';
const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { "content-type": "application/json" } });

function setup(glotUp = true) {
  const calls: { url: string; headers: Headers }[] = [];
  const f = (async (input: Request | string, init?: RequestInit) => {
    const u = new URL(String(typeof input === "string" ? input : input.url));
    calls.push({ url: u.pathname, headers: new Headers(init?.headers) });
    if (!glotUp) throw new Error("down");
    if (u.pathname.endsWith("/status")) return json({ enabled: true, off: [], versions: { es: 4 } });
    if (u.pathname.endsWith("/tm")) return json({ v: 4, t: {} });
    if (u.pathname.endsWith("/es/bundle")) {
      assert.equal(new Headers(init?.headers).get("x-glot-key"), "pk_x");
      assert.equal(new TextDecoder().decode(init?.body as ArrayBuffer), SRC);
      return new Response(OUT, { headers: { etag: '"glot-abc"', "x-glot-bundle": "1", "x-glot-version": "es@4" } });
    }
    return new Response("{}", { status: 404 });
  }) as typeof fetch;
  const store = new Map<string, Response>();
  const pageCache: PageCache = { match: async (k) => store.get(k)?.clone(), put: async (k, r) => void store.set(k, r) };
  const edge = createEdge({ server: "https://glot", project: "site", key: "pk_x", source: "en", translate: (html) => ({ html, total: 0, translated: 0, missing: [] }), fetch: f, timeout: 100, pageCache });
  const origin: { seen: Headers[] } & ((r: Request) => Promise<Response>) = Object.assign(
    async (r: Request) => {
      origin.seen.push(r.headers);
      return new Response(SRC, { headers: { "content-type": "text/javascript", "cache-control": "public, max-age=31536000, immutable", etag: '"origin-1"' } });
    },
    { seen: [] as Headers[] },
  );
  return { edge, calls, origin, store };
}

const chunk = (h: Record<string, string> = {}) => new Request("https://s/_next/static/chunks/page-1.js", { headers: h });

test("referer decides the chunk's language", () => {
  const o = { source: "en", locales: ["es", "de"] };
  const u = new URL("https://s/_next/static/chunks/a.js");
  assert.equal(refererLocale(chunk({ referer: "https://s/es/pricing" }), u, o), "es");
  assert.equal(refererLocale(chunk({ referer: "https://s/pricing" }), u, o), "en");
  assert.equal(refererLocale(chunk({ referer: "https://evil/es/x" }), u, o), "en");
  assert.equal(refererLocale(chunk(), u, o), "en");
});

test("localized chunk: glot's rewrite, no-cache + ETag, cached per version, 304, origin validators dropped", async () => {
  const { edge, origin, store, calls } = setup();
  const pending: Promise<unknown>[] = [];
  const r = await edge.bundle(chunk({ "if-none-match": '"origin-1"' }), "es", origin, (p) => void pending.push(p));
  assert.equal(r.status, 200);
  assert.equal(await r.text(), OUT);
  assert.equal(r.headers.get("cache-control"), "public, no-cache");
  assert.equal(r.headers.get("etag"), '"glot-abc"');
  assert.equal(origin.seen[0]!.get("if-none-match"), null, "the origin never answers 304 for a localized chunk");
  await Promise.all(pending);
  assert.deepEqual([...store.keys()], ["https://glot-edge.invalid/_bundle/es.4/_next/static/chunks/page-1.js"]);
  const posts = calls.filter((c) => c.url.endsWith("/bundle")).length;
  const again = await edge.bundle(chunk({ "if-none-match": '"glot-abc"' }), "es", origin, (p) => void pending.push(p));
  assert.equal(again.status, 304);
  assert.equal(calls.filter((c) => c.url.endsWith("/bundle")).length, posts, "served from the cache");
});

test("source pages: origin chunk, but revalidated instead of immutable", async () => {
  const { edge, origin } = setup();
  const r = await edge.bundle(chunk({ "if-none-match": '"origin-1"' }), "en", origin);
  assert.equal(await r.text(), SRC);
  assert.equal(r.headers.get("cache-control"), "public, no-cache");
  assert.equal(r.headers.get("etag"), '"origin-1"');
  assert.equal(origin.seen[0]!.get("if-none-match"), '"origin-1"');
});

test("glot down: origin chunk, no-store, no validators (fail-open)", async () => {
  const { edge, origin } = setup(false);
  const r = await edge.bundle(chunk(), "es", origin);
  assert.equal(r.status, 200);
  assert.equal(await r.text(), SRC);
  assert.match(r.headers.get("cache-control") ?? "", /no-store|no-cache/);
});
