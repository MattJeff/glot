// Segmented copy experiments, real Next 16 production build. The edge (`@glot/edge`) is simulated by sending the
// `x-glot-xp` request header it would set (contract §7). Checks: SSR HTML already in the variant (Server AND Client
// Components), no hydration error, and `track("purchase", {value, currency})` credits the assigned experiment/cell/arm.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";
import { chromium, type Browser } from "playwright";
import { writeXpHeader } from "@glot/runtime/experiments";

const PORT = Number(process.env.PORT ?? 3460), url = `http://localhost:${PORT}/`;
let next: ChildProcess, browser: Browser;

const a = [{ id: "11111111-1111-4111-8111-111111111111", cell: "22222222-2222-4222-8222-222222222222", arm: 1 }];
const xp = writeXpHeader({ v: 1, locale: "en", a, o: { "home.title": "Hi {name}, meet glot", "counter.button": "Click me" }, k: { [a[0].id]: ["home.title", "counter.button"] } });

before(async () => {
  next = spawn(resolve(import.meta.dirname, "../node_modules/.bin/next"), ["start", "-p", String(PORT)], {
    cwd: resolve(import.meta.dirname, ".."), stdio: "inherit",
  });
  for (let i = 0; ; i++) {
    try { if ((await fetch(url)).ok) break; } catch { /* not up yet */ }
    if (i > 100) throw new Error("next start did not come up");
    await new Promise((r) => setTimeout(r, 200));
  }
  browser = await chromium.launch();
});
after(async () => { await browser?.close(); next?.kill(); });

test("SSR HTML follows x-glot-xp (Server + Client Components); no header or wrong locale = official", async () => {
  const html = await (await fetch(url, { headers: { "x-glot-xp": xp } })).text();
  assert.match(html, /Hi <!-- -->Ana<!-- -->, meet glot|Hi Ana, meet glot/);
  assert.match(html, /Click me/);
  assert.doesNotMatch(html, /Welcome to glot/);
  assert.doesNotMatch(html, /visibility:hidden/, "edge path: <GlotCopy> never masks");

  const off = await (await fetch(url)).text();
  assert.match(off, /Welcome to glot/);
  assert.match(off, /Add one/);
  assert.match(off, /data-glot-key="home.tagline" style="visibility:hidden;animation:glot-reveal 0s 300ms forwards"/, "client-only: only that block is masked");

  const de = writeXpHeader({ v: 1, locale: "de", a, o: { "home.title": "Hallo {name}" } });
  assert.match(await (await fetch(url, { headers: { "x-glot-xp": de } })).text(), /Welcome to glot/);
  assert.match(await (await fetch(url, { headers: { "x-glot-xp": "garbage" } })).text(), /Welcome to glot/);
});

test("browser: variant hydrates without error, purchase credited to the edge's assignment with revenue", async () => {
  const ctx = await browser.newContext({ locale: "en-US", extraHTTPHeaders: { "x-glot-xp": xp } });
  const p = await ctx.newPage();
  const errors: string[] = [];
  p.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(m.text()); });
  p.on("pageerror", (e) => errors.push(String(e)));
  const hits: Record<string, unknown>[] = [], batches: unknown[] = [];
  await p.route("**/api/insights/pk_demo**", (r) => {
    const body = JSON.parse(r.request().postData() || "{}");
    (Array.isArray(body) ? batches : hits).push(body);
    return r.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await p.goto(url);
  await p.waitForLoadState("networkidle");

  assert.equal(await p.textContent("h1"), "Hi Ana, meet glot");
  assert.equal(await p.textContent("button"), "Click me");
  await p.click("button");
  await p.waitForFunction(() => document.querySelector("#count")?.textContent === "Clicks: 1", null, { timeout: 5000 });
  assert.deepEqual(await p.evaluate(() => JSON.parse(sessionStorage.getItem("glot_xp")!)), a);

  await p.click("#buy");
  for (let i = 0; i < 50 && !hits.some((h) => h.event); i++) await new Promise((r) => setTimeout(r, 100));
  const conv = hits.find((h) => h.event === "purchase");
  assert.ok(conv, JSON.stringify(hits));
  assert.deepEqual(conv.exposures, a);
  assert.equal(conv.value, 49);
  assert.equal(conv.currency, "EUR");
  assert.equal(hits.find((h) => !h.event)?.exposures, undefined, "the visit carries no exposure (counted by the edge)");

  // "Seen": the h1 (data-glot-key="home.title") has been on screen > 1 s; leaving the tab reports it once.
  await p.waitForTimeout(1200);
  const hide = () => p.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  });
  await hide();
  await hide();
  for (let i = 0; i < 30 && !batches.length; i++) await new Promise((r) => setTimeout(r, 100));
  assert.equal(batches.length, 1, "once per session and experiment");
  const [[{ served, exposures: [seen] }]] = batches as [[{ served: string; exposures: Record<string, unknown>[] }]];
  assert.equal(served, "en");
  assert.deepEqual({ ...seen, visible_ms: 0 }, { ...a[0], seen: true, visible_ms: 0 });
  assert.ok([1000, 2000, 5000].includes(seen.visible_ms as number), `bucketed: ${seen.visible_ms}`);

  assert.deepEqual(errors, []);
  await ctx.close();
});

test("client-only fallback: masked block is revealed within the budget, rest of the page never hidden", async () => {
  const ctx = await browser.newContext({ locale: "en-US" });
  const p = await ctx.newPage();
  const errors: string[] = [];
  p.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  await p.goto(url);
  assert.equal(await p.evaluate(() => getComputedStyle(document.querySelector("h1")!).visibility), "visible");
  await p.waitForFunction(() => getComputedStyle(document.querySelector("[data-glot-key]")!).visibility === "visible", null, { timeout: 1000 });
  assert.equal(await p.textContent("#tagline"), "Copy experiments without flicker");
  assert.deepEqual(errors, []);
  await ctx.close();
});

test("edge locale ≠ served locale: official text, assignment not stored, conversion not attributed", async () => {
  const de = writeXpHeader({ v: 1, locale: "de", a, o: { "home.title": "Hallo {name}" } });
  const ctx = await browser.newContext({ locale: "en-US", extraHTTPHeaders: { "x-glot-xp": de } });
  const p = await ctx.newPage();
  const hits: Record<string, unknown>[] = [];
  await p.route("**/api/insights/pk_demo**", (r) => {
    hits.push(JSON.parse(r.request().postData() || "{}"));
    return r.fulfill({ status: 200, contentType: "application/json", body: "{}" });
  });
  await p.goto(url);
  await p.waitForLoadState("networkidle");
  assert.equal(await p.textContent("h1"), "Welcome to glot, Ana");
  assert.equal(await p.evaluate(() => sessionStorage.getItem("glot_xp")), null);
  await p.click("#buy");
  for (let i = 0; i < 50 && !hits.some((h) => h.event); i++) await new Promise((r) => setTimeout(r, 100));
  const conv = hits.find((h) => h.event === "purchase");
  assert.ok(conv);
  assert.equal(conv.exposures, undefined);
  await ctx.close();
});
