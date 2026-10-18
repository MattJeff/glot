// decide(): hashing, weights, segments, holdout, persistence, bots, fallback + loader, Next proxy, Worker, bench.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readXpHeader } from "../../runtime/src/experiments.ts";
import { context, createExperimentsLoader, decide, glotExperiments, hash, matches, pick, type XpConfig } from "../src/experiments.ts";

const DE = "11111111-0000-0000-0000-000000000001";
const XP = "22222222-0000-0000-0000-000000000002";
const HO = "33333333-0000-0000-0000-000000000003";
const now = Date.parse("2026-10-18T12:00:00Z");
const cfg = (over: Partial<XpConfig> = {}): XpConfig => ({
  v: 1, generated_at: "2026-10-18T12:00:00Z", max_stale: 86400, persistence: "none", holdout: null,
  segments: [{ id: DE, rule: { all: [{ attr: "country", in: ["DE", "AT"] }, { attr: "source", eq: "search" }] } }],
  experiments: [{
    id: XP, status: "running", traffic_pct: 100, locales: ["en", "de"],
    cells: [{ id: DE, weights: [0, 1], count: true }, { id: "*", weights: [0.2, 0.8], count: true }],
    arms: [{ arm: 0 }, { arm: 1, values: { de: { "home.title": "Über 2 000 Teams" }, en: { "home.title": "2,000 teams" } } }],
  }],
  ...over,
});
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 Safari/605.1.15";
const req = (h: Record<string, string> = {}, url = "https://s/") => new Request(url, { headers: { "user-agent": UA, accept: "text/html,*/*", ...h } });
const unhead = (s: string) => JSON.parse(Buffer.from(s, "base64url").toString("utf8"));

test("hash: deterministic, uniform-ish, unit-stable decisions", () => {
  assert.equal(hash("glot:x:42"), hash("glot:x:42"));
  assert.notEqual(hash("a"), hash("b"));
  const a = decide(req(), cfg(), { locale: "en", unit: "visitor-1", now });
  const b = decide(req(), cfg(), { locale: "en", unit: "visitor-1", now });
  assert.deepEqual(a.assignments, b.assignments);
  assert.equal(a.header, b.header);
});

test("pick: distribution follows weights (100k units, ±1 %)", () => {
  const n = 100_000, hits = [0, 0, 0];
  for (let i = 0; i < n; i++) hits[pick([0.2, 0.5, 0.3], hash(`s:x:${i}`))]++;
  [0.2, 0.5, 0.3].forEach((w, i) => assert.ok(Math.abs(hits[i] / n - w) < 0.01, `${i}: ${hits[i] / n}`));
  assert.equal(pick([0, 0], 0.5), -1);
  assert.equal(pick([0, 1, 0], 0.999999), 1);
  // through decide(): '*' cell is 20/80
  let b = 0;
  for (let i = 0; i < 20_000; i++) b += decide(req(), cfg(), { locale: "en", unit: String(i), now }).assignments[0].arm;
  assert.ok(Math.abs(b / 20_000 - 0.8) < 0.015, String(b / 20_000));
});

test("context: country / device / source / visitor", () => {
  const c = context(req({ "cf-ipcountry": "de", "sec-ch-ua-mobile": "?1", referer: "https://www.google.com/" }), { locale: "de" });
  assert.deepEqual([c.country, c.device, c.source, c.visitor, c.locale], ["DE", "mobile", "search", "unknown", "de"]);
  const ipad = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)";
  assert.equal(context(req({ "user-agent": ipad }), { locale: "en" }).device, "tablet");
  assert.equal(context(req({ "x-vercel-ip-country": "XX" }), { locale: "en" }).country, undefined);
  const src = (url: string, ref?: string) => context(req(ref ? { referer: ref } : {}, url), { locale: "en" }).source;
  assert.equal(src("https://s/?utm_medium=cpc&utm_source=google"), "paid");
  assert.equal(src("https://s/?gclid=1"), "paid");
  assert.equal(src("https://s/?utm_source=newsletter&utm_medium=email"), "email");
  assert.equal(src("https://s/?utm_source=linkedin"), "social");
  assert.equal(src("https://s/", "https://t.co/abc"), "social");
  assert.equal(src("https://s/", "https://news.ycombinator.com/"), "referral");
  assert.equal(src("https://s/", "https://s/pricing"), "direct");
  assert.equal(src("https://s/"), "direct");
  const consent = (h: Record<string, string>) => context(req(h), { locale: "en", consent: true }).visitor;
  assert.equal(consent({}), "new");
  assert.equal(consent({ cookie: "glot_seen=1" }), "returning");
  assert.equal(context(req({ "cf-ipcountry": "DE" }), { locale: "en", attrs: { plan: "pro" } }).plan, "pro");
});

test("matches: contract §3 semantics", () => {
  assert.ok(matches({ all: [{ attr: "plan", not_in: ["free"] }] }, {})); // absent ⇒ not_in true
  assert.ok(!matches({ all: [{ attr: "plan", eq: "pro" }] }, {}));
  assert.ok(matches({ all: [{ attr: "plan", in: ["pro", "team"] }, { attr: "country", eq: "FR" }] }, { plan: "team", country: "FR" }));
});

test("decide: segment cell, overlay for served locale, header format", () => {
  const d = decide(req({ "cf-ipcountry": "AT", referer: "https://www.bing.com/" }), cfg(), { locale: "de", now });
  assert.deepEqual(d.assignments, [{ id: XP, cell: DE, arm: 1 }]);
  assert.deepEqual(d.exposures, d.assignments);
  assert.deepEqual(unhead(d.header!), { v: 1, locale: "de", a: [{ id: XP, cell: DE, arm: 1 }], o: { "home.title": "Über 2 000 Teams" } });
  assert.deepEqual(d.cookies, []); // persistence none
  // locale not covered ⇒ ignored
  assert.equal(decide(req(), cfg(), { locale: "it", now }).header, null);
  // arm 0 ⇒ assignment but no overlay
  const c = cfg();
  c.experiments[0].cells[1].weights = [1, 0];
  const z = decide(req(), c, { locale: "en", now });
  assert.deepEqual(unhead(z.header!).o, {});
});

test("decide: fallback (no config, stale, malformed version), opt-out, preview", () => {
  for (const d of [
    decide(req(), null, { locale: "en", now }),
    decide(req(), cfg(), { locale: "en", now: now + 86401_000 }),
    decide(req(), { ...cfg(), v: 2 as 1 }, { locale: "en", now }),
    decide(req({ "sec-gpc": "1" }), cfg(), { locale: "en", now }),
  ]) assert.deepEqual([d.header, d.exposures.length], [null, 0]);
  const p = decide(req({}, `https://s/?glot_xp=${XP}.1`), cfg({ persistence: "session" }), { locale: "en", now });
  assert.deepEqual([p.assignments[0].arm, p.exposures.length, p.cookies.length], [1, 0, 0]);
});

test("decide: bots get the same treatment, never counted; prefetch not counted", () => {
  const bot = { "user-agent": "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)" };
  const h = decide(req(), cfg(), { locale: "en", unit: "u", now });
  const b = decide(req(bot), cfg(), { locale: "en", unit: "u", now });
  assert.deepEqual(b.assignments, h.assignments); // no cloaking
  assert.equal(b.header, h.header);
  assert.equal(b.exposures.length, 0);
  assert.equal(decide(req({ "next-router-prefetch": "1" }), cfg(), { locale: "en", now }).exposures.length, 0);
  assert.equal(decide(req({ "user-agent": "" }), cfg(), { locale: "en", now }).exposures.length, 0);
});

test("decide: only document navigations are exposures; RSC / prefetch get the same decision", () => {
  const doc = decide(req(), cfg(), { locale: "en", unit: "u", now });
  assert.equal(doc.exposures.length, 1);
  for (const [h, url] of [
    [{ rsc: "1", accept: "*/*" }, "https://s/"],
    [{ "next-router-state-tree": "%5B%5D", accept: "*/*" }, "https://s/"],
    [{ accept: "*/*" }, "https://s/x?_rsc=abc"],
    [{ "next-router-prefetch": "1" }, "https://s/"],
    [{ purpose: "prefetch" }, "https://s/"],
    [{ "sec-purpose": "prefetch;prerender", "sec-fetch-dest": "document" }, "https://s/"],
    [{ "sec-fetch-dest": "empty" }, "https://s/"],
    [{ accept: "application/json" }, "https://s/"],
  ] as const) {
    const d = decide(req(h, url), cfg(), { locale: "en", unit: "u", now });
    assert.equal(d.header, doc.header, JSON.stringify(h));
    assert.equal(d.exposures.length, 0, JSON.stringify(h));
  }
  assert.equal(decide(req({ "sec-fetch-dest": "document", accept: "*/*" }), cfg(), { locale: "en", now }).exposures.length, 1);
  // session: an arm already in the cookie was counted on its first page
  const c = cfg({ persistence: "session", holdout: { id: HO, pct: 0 } });
  const first = decide(req(), c, { locale: "en", now, rand: () => 0.9 });
  assert.equal(first.exposures.length, 2);
  const again = decide(req({ cookie: first.cookies[0].split(";")[0] }), c, { locale: "en", now, rand: () => 0.1 });
  assert.deepEqual([again.assignments, again.exposures.length], [first.assignments, 0]);
});

test("decide: x-glot-xp.k (contract v1.6) = keys of measurable experiments, read back by @glot/runtime", () => {
  const keys = ["home.title", "home.cta"];
  const c = cfg({ holdout: { id: HO, pct: 0 } });
  c.experiments[0].keys = keys;
  const d = decide(req(), c, { locale: "en", now, rand: () => 0.1 });
  const x = readXpHeader(d.header);
  assert.deepEqual(x?.k, { [XP]: keys }); // holdout has no keys
  assert.deepEqual(x?.a, d.assignments);
  assert.deepEqual(x?.o, d.overlay);
  // no keys in the config / preview / personalized cell ⇒ no `k` (header stays as before)
  assert.equal(unhead(decide(req(), cfg(), { locale: "en", now }).header!).k, undefined);
  assert.equal(unhead(decide(req({}, `https://s/?glot_xp=${XP}.1`), c, { locale: "en", now }).header!).k, undefined);
  c.experiments[0].cells[1].count = false;
  assert.equal(unhead(decide(req(), c, { locale: "en", now, rand: () => 0.1 }).header!).k, undefined);
});

test("decide: holdout first, personalized cell served not counted", () => {
  const ho = cfg({ holdout: { id: HO, pct: 100 } });
  const d = decide(req(), ho, { locale: "en", now });
  assert.deepEqual(d.assignments, [{ id: HO, cell: "*", arm: 0 }]);
  assert.deepEqual(unhead(d.header!).o, {});
  const out = decide(req(), cfg({ holdout: { id: HO, pct: 0 } }), { locale: "en", now, rand: () => 0.1 });
  assert.deepEqual(out.assignments.map((a) => a.id), [HO, XP]);
  const pz = cfg();
  pz.experiments[0].cells[1] = { id: "*", weights: [0, 1], count: false };
  const p = decide(req(), pz, { locale: "en", now });
  assert.deepEqual([p.assignments.length, p.exposures.length], [1, 0]);
});

test("decide: session cookie sticks, re-draws an arm no longer served, consent cookie", () => {
  const c = cfg({ persistence: "session" });
  const first = decide(req(), c, { locale: "en", now, rand: () => 0.1 }); // 0.1 < 0.2 ⇒ arm 0
  assert.equal(first.assignments[0].arm, 0);
  assert.equal(first.cookies[0], `glot_xp=${XP}.0.*; Path=/; SameSite=Lax; Secure`);
  const again = decide(req({ cookie: `glot_xp=${XP}.0.*` }), c, { locale: "en", now, rand: () => 0.9 });
  assert.deepEqual([again.assignments[0].arm, again.cookies.length], [0, 0]); // sticky, no Set-Cookie
  c.experiments[0].cells[1].weights = [0, 1]; // arm 0 no longer served
  const re = decide(req({ cookie: `glot_xp=${XP}.0.*` }), c, { locale: "en", now, rand: () => 0.1 });
  assert.equal(re.assignments[0].arm, 1);
  const it = decide(req({ cookie: `glot_xp=${XP}.1.*` }), cfg({ persistence: "session", holdout: { id: HO, pct: 0 } }), { locale: "it", now });
  assert.equal(it.cookies[0], `glot_xp=${HO}.1.*~${XP}.1.*; Path=/; SameSite=Lax; Secure`); // other-locale arm kept
  assert.match(decide(req(), cfg(), { locale: "en", now, consent: true }).cookies[0], /^glot_seen=1; Path=\/; Max-Age=31536000/);
  assert.equal(decide(req(), cfg(), { locale: "en", now }).cookies.length, 0); // no consent ⇒ no persistent cookie
});

test("decide: max_stale counts from the last 200/304, not generated_at", () => {
  const old = cfg({ generated_at: "2020-01-01T00:00:00Z" });
  assert.equal(decide(req(), old, { locale: "en", now }).header, null);
  assert.ok(decide(req(), { ...old, fetched_at: now - 1000 }, { locale: "en", now }).header);
});

test("loader: cold-start timeout → null, then cached, SWR, glot down keeps last config", async () => {
  let calls = 0, delay = 300, fail = false;
  const f = (async (_u: string, init?: RequestInit) => {
    calls++;
    if (fail) throw new Error("down");
    await new Promise((r) => setTimeout(r, delay));
    if ((init?.headers as Record<string, string>)?.["if-none-match"] === '"1"') return new Response(null, { status: 304 });
    return new Response(JSON.stringify(cfg()), { headers: { etag: '"1"' } });
  }) as typeof fetch;
  const load = createExperimentsLoader({ server: "https://glot/", project: "acme", ttl: 0, timeout: 50, fetch: f });
  const pending: Promise<unknown>[] = [];
  assert.equal(await load((p) => pending.push(p)), null); // glot slow: official text, never blocks > timeout
  await Promise.all(pending);
  delay = 0;
  assert.equal((await load())?.v, 1); // cached (SWR fires a 304 revalidation in the background)
  fail = true;
  await new Promise((r) => setTimeout(r, 5));
  assert.equal((await load())?.v, 1);
  assert.ok(calls >= 2);
});

test("glotExperiments: exposures sent once in the background, one Hit per document", async () => {
  const posts: unknown[] = [];
  const f = (async (u: string, init?: RequestInit) => {
    if (u.endsWith("/experiments.json")) return new Response(JSON.stringify({ ...cfg(), generated_at: new Date().toISOString() }));
    posts.push([u, JSON.parse(String(init?.body))]);
    return new Response("{}");
  }) as typeof fetch;
  const xp = glotExperiments({ server: "https://glot", project: "acme", key: "pk_1", fetch: f, timeout: 1000, batchMs: 0 });
  const bg: Promise<unknown>[] = [];
  const d = await xp(req({ "cf-ipcountry": "DE", referer: "https://google.de/" }), "de", (p) => bg.push(p));
  await Promise.all(bg);
  assert.deepEqual(posts, [["https://glot/api/insights/pk_1/batch", [{ served: "de", exposures: [{ id: XP, cell: DE, arm: 1 }] }]]]);
  assert.ok(d.header);
});

test("next proxy: x-glot-xp forwarded (forged one dropped), private, cookie", async () => {
  const { glotProxy } = await import("../src/next-proxy.ts");
  const { NextRequest } = await import("next/server.js");
  const f = (async (u: string) =>
    u.endsWith("/experiments.json")
      ? new Response(JSON.stringify({ ...cfg({ persistence: "session" }), generated_at: new Date().toISOString() }))
      : new Response("{}")) as typeof fetch;
  const proxy = glotProxy({ source: "en", locales: ["de"], experiments: { server: "https://glot", project: "acme", key: "pk", fetch: f, timeout: 1000 } });
  const res = await proxy(new NextRequest("https://s/de/x", { headers: { "user-agent": UA, accept: "text/html", "x-glot-xp": "forged", "cf-ipcountry": "DE", referer: "https://google.com/" } }));
  const fwd = res.headers.get("x-middleware-request-x-glot-xp")!;
  assert.deepEqual(unhead(fwd).a, [{ id: XP, cell: DE, arm: 1 }]);
  assert.equal(res.headers.get("x-middleware-request-x-glot-locale"), "de");
  assert.equal(res.headers.get("cache-control"), "private, no-store");
  assert.match(res.headers.get("set-cookie") ?? "", /^glot_xp=/);
});

test("bench: decide < 1 ms (p99 well under; reports mean)", () => {
  const c = cfg({ holdout: { id: HO, pct: 5 } });
  for (let i = 0; i < 4; i++) c.experiments.push({ ...c.experiments[0], id: `${XP}-${i}` });
  const r = req({ "cf-ipcountry": "DE", referer: "https://www.google.com/", "sec-ch-ua-mobile": "?0", cookie: "a=b" }, "https://s/pricing?utm_source=x");
  for (let i = 0; i < 2000; i++) decide(r, c, { locale: "de", now });
  const n = 20_000, t = performance.now();
  for (let i = 0; i < n; i++) decide(r, c, { locale: "de", now });
  const mean = (performance.now() - t) / n;
  console.log(`decide(): ${(mean * 1000).toFixed(1)} µs/decision (5 experiments + holdout)`);
  assert.ok(mean < 1, `${mean} ms`);
});

test("load: 1000 concurrent documents through glotExperiments", async () => {
  let fetches = 0, posts = 0, items = 0;
  const f = (async (u: string, init?: RequestInit) => {
    if (u.endsWith("/experiments.json")) return fetches++, new Response(JSON.stringify({ ...cfg(), generated_at: new Date().toISOString() }));
    items += JSON.parse(String(init?.body)).length;
    return posts++, new Response("{}");
  }) as typeof fetch;
  const xp = glotExperiments({ server: "https://glot", project: "acme", key: "pk", fetch: f, timeout: 1000 });
  const bg: Promise<unknown>[] = [];
  const t = performance.now();
  const ds = await Promise.all(Array.from({ length: 1000 }, (_, i) => xp(req({ "cf-ipcountry": i % 2 ? "DE" : "FR" }), "en", (p) => bg.push(p))));
  await Promise.all(bg);
  console.log(`1000 documents: ${(performance.now() - t).toFixed(0)} ms, ${fetches} config fetch, ${posts} batch posts`);
  assert.equal(fetches, 1); // deduped in flight
  assert.equal(items, 1000);
  assert.ok(posts <= 21, String(posts)); // batches of ≤ 50
  assert.ok(ds.every((d) => d.header));
});
