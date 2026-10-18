// On-demand locales end to end: a real glot-server (mock LLM, fresh Postgres database, :8797) + the Next app
// started with GLOT_SERVER. First `de` visit: English HTML + on-device translation in the browser (and glot
// queues the job); once the job is done, the SERVER HTML itself is German (lang="de"), no on-device pass.
// Needs Postgres on :5433 and `psql`; builds glot-server into target/integ unless GLOT_SERVER_BIN is set.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";
import { chromium, type Browser } from "playwright";
import en from "../messages/en.json" with { type: "json" };

const root = resolve(import.meta.dirname, "../../..");
const PG = process.env.DATABASE_URL ?? "postgres://glot:glot@localhost:5433/glot";
const GLOT = "http://127.0.0.1:8797", PORT = 3242, url = `http://localhost:${PORT}/`;
const db = `glot_e2e_${Date.now()}`;
let glot: ChildProcess, next: ChildProcess, browser: Browser;

const fake = `(() => {
  const dict = { Welcome: "Willkommen", to: "bei", Ana: "Ana" };
  window.Translator = {
    async availability() { return "available"; },
    async create() { return { async translate(s) { return s.replace(/[A-Za-z]+/g, (w) => dict[w] ?? w); } }; },
  };
})()`;

async function up(u: string) {
  for (let i = 0; ; i++) {
    try { if ((await fetch(u)).ok) return; } catch { /* not up yet */ }
    if (i > 150) throw new Error(`${u} did not come up`);
    await new Promise((r) => setTimeout(r, 200));
  }
}

before(async () => {
  let bin = process.env.GLOT_SERVER_BIN;
  if (!bin) {
    const target = process.env.CARGO_TARGET_DIR ?? resolve(root, "target/integ");
    execFileSync("cargo", ["build", "-p", "glot-server"], { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: target } });
    bin = resolve(target, "debug/glot-server");
  }
  execFileSync("psql", [PG, "-qc", `CREATE DATABASE ${db}`]);
  glot = spawn(bin, [], { stdio: "inherit", env: { ...process.env, DATABASE_URL: PG.replace(/\/[^/]*$/, `/${db}`), GLOT_BIND: "127.0.0.1:8797", GLOT_SECRET_KEY: "e2e", RUST_LOG: "warn" } });
  await up(`${GLOT}/healthz`);

  // Project with the app's English keys, public CDN, mock LLM, `de` allowed on demand.
  const res = await fetch(`${GLOT}/api/auth/signup`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ email: "e2e@x.io", password: "correct horse", name: "E" }) });
  const cookie = res.headers.get("set-cookie")!.split(";")[0];
  const api = async (method: string, path: string, body: unknown) => {
    const r = await fetch(`${GLOT}/api${path}`, { method, headers: { cookie, "content-type": "application/json" }, body: JSON.stringify(body) });
    assert.ok(r.ok, `${method} ${path}: ${r.status} ${await r.text()}`);
  };
  await api("POST", "/projects", { name: "Next", slug: "runtime-next", source_locale: "en", target_locales: ["fr"] });
  await api("PATCH", "/projects/runtime-next", { public_cdn: true, llm: { provider: "mock", model: "mock" } });
  for (const [ns, msgs] of Object.entries(en)) for (const [k, source] of Object.entries(msgs)) await api("POST", "/projects/runtime-next/keys", { key: `${ns}.${k}`, source });
  await api("PUT", "/projects/runtime-next/ondemand", { locales: ["de"], monthly_budget_usd: 1, per_day: 5 });

  next = spawn(resolve(import.meta.dirname, "../node_modules/.bin/next"), ["start", "-p", String(PORT)], {
    cwd: resolve(import.meta.dirname, ".."), stdio: "inherit", env: { ...process.env, GLOT_SERVER: GLOT, GLOT_PROJECT: "runtime-next" },
  });
  await up(url);
  browser = await chromium.launch();
});

after(async () => {
  await browser?.close();
  next?.kill();
  glot?.kill();
  await new Promise((r) => setTimeout(r, 300));
  try { execFileSync("psql", [PG, "-qc", `DROP DATABASE IF EXISTS ${db} WITH (FORCE)`]); } catch { /* best effort */ }
});

test("first de visit: English server HTML + on-device; then German server HTML", async () => {
  const ctx = await browser.newContext({ locale: "de-DE" });
  const p = await ctx.newPage();
  await p.addInitScript(fake);
  const first = await (await p.goto(url))!.text();
  assert.match(first, /<html lang="en"/);
  assert.match(first, /Welcome to glot, Ana/);
  await p.waitForFunction(() => document.querySelector("h1")?.textContent === "Willkommen bei glot, Ana");
  await ctx.close();

  // glot translates in the background (mock LLM: "⟦de⟧ …"); the next server render is German.
  let html = "";
  for (let i = 0; i < 100 && !html.includes('lang="de"'); i++) {
    await new Promise((r) => setTimeout(r, 300));
    html = await (await fetch(url, { headers: { "accept-language": "de-DE,de;q=0.9" } })).text();
  }
  assert.match(html, /<html lang="de"/);
  assert.match(html, /⟦de⟧ Welcome to glot, Ana/);
  assert.match(html, /⟦de⟧ This text comes from a Server Component/);
  // ETag'd catalog on the glot CDN.
  assert.ok((await fetch(`${GLOT}/cdn/runtime-next/de.json`)).headers.get("etag"));

  // Browser: the server's German stays (de counts as shipped now: no badge, no hydration error).
  const ctx2 = await browser.newContext({ locale: "de-DE" });
  const p2 = await ctx2.newPage();
  const errors: string[] = [];
  p2.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  p2.on("pageerror", (e) => errors.push(String(e)));
  await p2.addInitScript(fake);
  await p2.goto(url);
  await p2.waitForLoadState("networkidle");
  assert.equal(await p2.textContent("h1"), "⟦de⟧ Welcome to glot, Ana");
  assert.match((await p2.textContent("#count"))!, /⟦de⟧/);
  assert.equal(await p2.$("#glot-runtime-badge"), null);
  assert.deepEqual(errors, []);
  await ctx2.close();

  // English visitors are untouched.
  assert.match(await (await fetch(url, { headers: { "accept-language": "en-US" } })).text(), /<html lang="en"/);
});
