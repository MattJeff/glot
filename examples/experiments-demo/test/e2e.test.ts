// Segmented copy experiments end to end: real glot-server (mock LLM, dedicated database glot_xpdemo, :8820) + this
// Next app behind glotProxy (edge decision) + withExperiments (SSR overlay) + track (conversions).
// An experiment on home.hero.{title,subtitle,cta}: 3 intentions, segments country DE / FR / everyone else.
// Needs Postgres on :5433 and `psql`; builds glot-server into target/integ unless GLOT_SERVER_BIN is set.
// Ports: 3491 app with experiments, 3492 app with glot unreachable (3498), 3493 app without experiments, 3499 CDP.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";
import { chromium, type Browser, type BrowserContext, type Page } from "playwright";
import en from "../messages/en.json" with { type: "json" };
import fr from "../messages/fr.json" with { type: "json" };
import de from "../messages/de.json" with { type: "json" };

const root = resolve(import.meta.dirname, "../../..");
const app = resolve(import.meta.dirname, "..");
const PG = process.env.DATABASE_URL ?? "postgres://glot:glot@localhost:5433/glot";
const DB = "glot_xpdemo";
const GLOT = "http://127.0.0.1:8820";
const XP = "http://localhost:3491", DOWN = "http://localhost:3492", BASE = "http://localhost:3493";
// Playwright's default UA says "HeadlessChrome", which the edge rightly treats as a bot (never counted).
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const GOOGLEBOT = "Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

type Locale = "en" | "fr" | "de";
type Hero = { "home.hero.title": string; "home.hero.subtitle": string; "home.hero.cta": string };
const pick = (m: typeof en): Hero => ({ "home.hero.title": m.home.hero.title, "home.hero.subtitle": m.home.hero.subtitle, "home.hero.cta": m.home.hero.cta });
const official: Record<Locale, Hero> = { en: pick(en), fr: pick(fr), de: pick(de) };
// Three intentions, written in every locale (only declared facts: "2,000 teams", "4.8/5").
const hero = (title: string, subtitle: string, cta: string): Hero => ({ "home.hero.title": title, "home.hero.subtitle": subtitle, "home.hero.cta": cta });
const intents: { intent: string; values: Record<Locale, Hero> }[] = [
  { intent: "social proof", values: {
    en: hero("Join 2,000 teams who get paid on time", "Freelancers and agencies rate Relay 4.8/5 for chasing invoices so they don't have to.", "Join them free"),
    fr: hero("Rejoignez 2 000 équipes payées à temps", "Indépendants et agences notent Relay 4,8/5 pour les relances qu'ils n'ont plus à faire.", "Les rejoindre"),
    de: hero("2.000 Teams werden pünktlich bezahlt", "Freelancer und Agenturen bewerten Relay mit 4,8/5, weil es das Mahnen übernimmt.", "Jetzt dabei sein") } },
  { intent: "direct benefit", values: {
    en: hero("Get paid faster, without chasing", "Relay sends the invoice and the reminders. You do the work you like.", "Get paid faster"),
    fr: hero("Soyez payé plus vite, sans relancer", "Relay envoie la facture et les relances. Vous faites le travail que vous aimez.", "Être payé plus vite"),
    de: hero("Schneller bezahlt, ohne Mahnen", "Relay verschickt Rechnung und Erinnerungen. Sie machen die Arbeit, die Sie mögen.", "Schneller bezahlt werden") } },
  { intent: "simplicity", values: {
    en: hero("Invoicing in two clicks", "Pick a client, check the amount, send. Relay handles the rest.", "Try it now"),
    fr: hero("La facturation en deux clics", "Choisissez un client, vérifiez le montant, envoyez. Relay gère le reste.", "Essayer maintenant"),
    de: hero("Rechnungen in zwei Klicks", "Kunde wählen, Betrag prüfen, senden. Relay erledigt den Rest.", "Jetzt ausprobieren") } },
];
const textOf = (arm: number, l: Locale) => (arm === 0 ? official[l] : intents[arm - 1].values[l]);

let glot: ChildProcess | undefined, browser: Browser;
const nexts: ChildProcess[] = [];
let xpId = "", seg: Record<"DE" | "FR", string> = { DE: "", FR: "" }, publicKey = "", cookie = "";
const errors: string[] = [];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function up(u: string) {
  for (let i = 0; ; i++) {
    try { if ((await fetch(u)).ok) return; } catch { /* not up yet */ }
    if (i > 150) throw new Error(`${u} did not come up`);
    await sleep(200);
  }
}
async function api(method: string, path: string, body?: unknown) {
  const r = await fetch(`${GLOT}/api${path}`, { method, headers: { cookie, "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await r.text();
  assert.ok(r.ok, `${method} ${path}: ${r.status} ${text}`);
  return text ? JSON.parse(text) : null;
}
const sql = (q: string) => execFileSync("psql", [PG.replace(/\/[^/]*$/, `/${DB}`), "-tAc", q], { encoding: "utf8" }).trim();
/** `{arm: n}` of xp_counts for an event, cell and the demo experiment. */
const counts = (event: string, cell: string) =>
  Object.fromEntries(sql(`SELECT arm, sum(n) FROM xp_counts WHERE experiment_id='${xpId}' AND cell='${cell}' AND event='${event}' GROUP BY arm`)
    .split("\n").filter(Boolean).map((l) => l.split("|").map(Number)));
const total = (c: Record<number, number>) => Object.values(c).reduce((a, b) => a + b, 0);

function startNext(port: number, env: Record<string, string>) {
  const p = spawn(resolve(app, "node_modules/.bin/next"), ["start", "-p", String(port)], { cwd: app, stdio: "inherit", env: { ...process.env, ...env } });
  nexts.push(p);
}

before(async () => {
  let bin = process.env.GLOT_SERVER_BIN;
  if (!bin) {
    const target = process.env.CARGO_TARGET_DIR ?? resolve(root, "target/integ");
    execFileSync("cargo", ["build", "-p", "glot-server"], { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: target } });
    bin = resolve(target, "debug/glot-server");
  }
  execFileSync("psql", [PG, "-qc", `DROP DATABASE IF EXISTS ${DB} WITH (FORCE)`]);
  execFileSync("psql", [PG, "-qc", `CREATE DATABASE ${DB}`]);
  glot = spawn(bin, [], { stdio: "inherit", env: { ...process.env, DATABASE_URL: PG.replace(/\/[^/]*$/, `/${DB}`), GLOT_BIND: "127.0.0.1:8820", GLOT_SECRET_KEY: "xpdemo", GLOT_WEB_ORIGIN: XP, GLOT_XP_TICK_SECS: "2", RUST_LOG: "warn" } });
  await up(`${GLOT}/healthz`);

  const res = await fetch(`${GLOT}/api/auth/signup`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ email: "xpdemo@relay.test", password: "correct horse battery", name: "Relay" }) });
  assert.ok(res.ok, `signup ${res.status}`);
  cookie = res.headers.get("set-cookie")!.split(";")[0];
  await api("POST", "/projects", { name: "Relay", slug: "relay", source_locale: "en", target_locales: ["fr", "de"] });
  await api("PATCH", "/projects/relay", { public_cdn: true, llm: { provider: "mock", model: "mock" } });
  const flat = (o: object, p = ""): [string, string][] => Object.entries(o).flatMap(([k, v]) => (typeof v === "string" ? [[p + k, v] as [string, string]] : flat(v, `${p}${k}.`)));
  for (const [key, source] of flat(en)) await api("POST", "/projects/relay/keys", { key, source });
  publicKey = (await api("GET", "/projects/relay/insights")).public_key;
  await api("PUT", "/projects/relay/xp/settings", { persistence: "session" });

  // The experiment, as the wizard / MCP would create it: hero group, 3 intentions, DE / FR / everyone else.
  // Fixed split without the official arm, so every visitor provably sees a variant (flicker = official text visible).
  const xp = await api("POST", "/projects/relay/xp", {
    name: "Hero by country", prefix: "home.hero", locales: ["en", "fr", "de"],
    cells: [{ name: "Germany", rule: { all: [{ attr: "country", in: ["DE"] }] } }, { name: "France", rule: { all: [{ attr: "country", in: ["FR"] }] } }],
    allocation: "fixed", weights: [0, 1, 1, 1], min_n: 50, goal: "signup", facts: ["2,000 teams", "2 000 équipes", "2.000 Teams", "4.8/5", "4,8/5"],
    arms: intents.map(({ intent, values }) => ({ intent, values })),
  });
  xpId = xp.id;
  for (const c of xp.cells) seg[c.name === "Germany" ? "DE" : "FR"] = c.id;
  await api("POST", `/projects/relay/xp/${xpId}/start`);
  const config = await (await fetch(`${GLOT}/cdn/relay/experiments.json`)).json();
  assert.equal(config.experiments[0]?.id, xpId, JSON.stringify(config));
  assert.equal(config.persistence, "session");

  startNext(3491, { GLOT_SERVER: GLOT, GLOT_PROJECT: "relay", GLOT_KEY: publicKey });
  startNext(3492, { GLOT_SERVER: "http://127.0.0.1:3498", GLOT_PROJECT: "relay", GLOT_KEY: publicKey });
  startNext(3493, {});
  await Promise.all([up(XP), up(DOWN), up(BASE)]);
  // Cold start: the first request may beat the config (150 ms budget → official text). Warm the isolate.
  for (let i = 0; i < 50; i++) {
    if (!(await (await fetch(XP, { headers: { "user-agent": "curl" } })).text()).includes(official.en["home.hero.title"])) break;
    await sleep(100);
  }
  browser = await chromium.launch({ args: ["--remote-debugging-port=3499"] });
});

after(async () => {
  await browser?.close();
  for (const p of nexts) p.kill();
  glot?.kill();
  await sleep(300);
  // KEEP_DB=1 keeps glot_xpdemo for inspection (dropped again at the next run).
  if (!process.env.KEEP_DB) try { execFileSync("psql", [PG, "-qc", `DROP DATABASE IF EXISTS ${DB} WITH (FORCE)`]); } catch { /* best effort */ }
});

async function visitor(o: { country?: string; ua?: string } = {}) {
  const ctx = await browser.newContext({ userAgent: o.ua ?? UA });
  // A CDN adds the geo header on the way to the app only (never on the beacon to glot): same here.
  if (o.country) await ctx.route(/^http:\/\/localhost:349\d\//, (r) => r.continue({ headers: { ...r.request().headers(), "cf-ipcountry": o.country! } }));
  return ctx;
}
function watch(p: Page) {
  p.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(`${p.url()}: ${m.text()}`); });
  p.on("pageerror", (e) => errors.push(`${p.url()}: ${e}`));
}
/** `{id, arm, cell}` of the demo experiment from the session cookie. */
async function assignment(ctx: BrowserContext) {
  const v = (await ctx.cookies()).find((c) => c.name === "glot_xp")?.value ?? "";
  const part = v.split("~").map((p) => p.split(".")).find(([id]) => id === xpId);
  assert.ok(part, `no glot_xp assignment for ${xpId} in "${v}"`);
  return { arm: Number(part[1]), cell: part[2], raw: v };
}
const heroText = (p: Page) => p.evaluate(() => ["hero-title", "hero-subtitle", "hero-cta"].map((id) => document.getElementById(id)?.textContent));

// Records every value #hero-title ever had, from the first parsed byte, and its text at the first animation frame.
const recorder = `(() => {
  const seen = (window.__seen = []);
  const rec = () => { const t = document.getElementById("hero-title")?.textContent; if (t != null && seen.at(-1) !== t) seen.push(t); };
  new MutationObserver(rec).observe(document, { subtree: true, childList: true, characterData: true });
  requestAnimationFrame(() => { window.__firstFrame = document.getElementById("hero-title")?.textContent ?? null; });
})()`;

test("(a) zero flicker: server HTML, first frame and every DOM state carry the variant", async () => {
  for (const [path, locale, country] of [["/", "en", "US"], ["/fr", "fr", "FR"], ["/de", "de", "DE"]] as const) {
    const ctx = await visitor({ country });
    const p = await ctx.newPage();
    watch(p);
    await p.addInitScript(recorder);
    const html = await (await p.goto(XP + path))!.text();
    const { arm } = await assignment(ctx);
    assert.ok(arm >= 1, "weights exclude the official arm");
    const want = textOf(arm, locale);
    // Server HTML: variant in, official text out (title, subtitle, CTA, and the nav CTA).
    for (const k of Object.keys(want) as (keyof Hero)[]) {
      assert.ok(html.includes(want[k].replace(/&/g, "&amp;").replace(/'/g, "&#x27;")), `${path}: ${k} variant missing from HTML`);
      assert.ok(!html.includes(official[locale][k]), `${path}: official ${k} in HTML`);
    }
    await p.waitForLoadState("networkidle");
    const seen = await p.evaluate(() => (window as unknown as { __seen: string[] }).__seen);
    const first = await p.evaluate(() => (window as unknown as { __firstFrame: string }).__firstFrame);
    assert.deepEqual(seen, [want["home.hero.title"]], `${path}: hero went through ${JSON.stringify(seen)}`);
    assert.equal(first, want["home.hero.title"], `${path}: first frame`);
    assert.deepEqual(await heroText(p), Object.values(want));
    await ctx.close();
  }
});

test("(b) session persistence: same visitor, same arm on every page, reloads and client navigation", async () => {
  const ctx = await visitor({ country: "US" });
  const p = await ctx.newPage();
  watch(p);
  await p.goto(XP);
  const a = await assignment(ctx);
  const cta = textOf(a.arm, "en")["home.hero.cta"];
  for (const path of ["/pricing", "/signup", "/", "/pricing", "/"]) {
    await p.goto(XP + path);
    assert.equal(await p.textContent("#nav-cta"), cta, path);
    assert.equal((await assignment(ctx)).raw, a.raw, `${path}: cookie changed`);
  }
  await p.click("text=" + (await p.textContent("nav a[href='/pricing']"))!);
  await p.waitForURL(/\/pricing$/);
  assert.equal(await p.textContent("#nav-cta"), cta, "client navigation");
  // A new browser session may draw another arm, never mixes keys of two arms.
  await ctx.close();
});

test("(c) segments: country headers pick the DE / FR / * cell, whatever the locale", async () => {
  const cases = [["DE", "/de", "de", seg.DE], ["DE", "/", "en", seg.DE], ["FR", "/fr", "fr", seg.FR], ["FR", "/de", "de", seg.FR], ["US", "/", "en", "*"], ["JP", "/fr", "fr", "*"], [undefined, "/", "en", "*"]] as const;
  for (const [country, path, locale, cell] of cases) {
    const ctx = await visitor({ country });
    const p = await ctx.newPage();
    watch(p);
    await p.goto(XP + path);
    const a = await assignment(ctx);
    assert.equal(a.cell, cell, `${country} ${path}`);
    assert.deepEqual(await heroText(p), Object.values(textOf(a.arm, locale)), `${country} ${path}`);
    await ctx.close();
  }
});

test("(d) conversions are attributed to the arm and cell seen, and visible through the results API", async () => {
  const before = { exp: counts("$exposure", seg.DE), conv: counts("signup", seg.DE) };
  const seen: Record<number, number> = {};
  const converted: Record<number, number> = {};
  for (let i = 0; i < 12; i++) {
    const ctx = await visitor({ country: "DE" });
    const p = await ctx.newPage();
    watch(p);
    await p.goto(XP + "/de");
    const { arm, cell } = await assignment(ctx);
    assert.equal(cell, seg.DE);
    seen[arm] = (seen[arm] ?? 0) + 1;
    if (i % 3 !== 2) {
      // 2 visitors out of 3 sign up (and one of them submits twice: still one conversion per session).
      await p.click("#hero-cta");
      await p.waitForURL(/\/de\/signup$/);
      const hit = p.waitForRequest((r) => r.url().startsWith(`${GLOT}/api/insights/`) && (r.postData() ?? "").includes('"signup"'));
      await p.fill("input[type=email]", `v${i}@example.com`);
      await p.click("button[type=submit]");
      await hit;
      await p.waitForSelector("#signup-done");
      converted[arm] = (converted[arm] ?? 0) + 1;
    }
    await p.waitForLoadState("networkidle");
    await ctx.close();
  }
  // Exposures are sent from the edge after the response (waitUntil): give them a moment.
  let exp: Record<number, number> = {};
  for (let i = 0; i < 40; i++) {
    exp = counts("$exposure", seg.DE);
    if (total(exp) - total(before.exp) >= 12) { await sleep(1000); exp = counts("$exposure", seg.DE); break; }
    await sleep(250);
  }
  const conv = counts("signup", seg.DE);
  for (const arm of [1, 2, 3]) {
    // Contract §6: one exposure per HTML document (the signup page is reached by client navigation, not a document).
    assert.equal((exp[arm] ?? 0) - (before.exp[arm] ?? 0), seen[arm] ?? 0, `arm ${arm}: exposures ${JSON.stringify(exp)} vs documents ${JSON.stringify(seen)}`);
    assert.equal((conv[arm] ?? 0) - (before.conv[arm] ?? 0), converted[arm] ?? 0, `arm ${arm}: conversions ${JSON.stringify(conv)} vs ${JSON.stringify(converted)}`);
  }
  assert.equal(total(counts("signup", "*")), 0, "no conversion leaked to another cell");

  // Results API (contract v1.2): analyses written by the rebalance worker (GLOT_XP_TICK_SECS=2 here).
  let latest: { global: { views: number }; cells: { name: string; views: number; arms: { mean: number }[] }[] } | null = null;
  for (let i = 0; i < 40 && !(latest && latest.cells.find((c) => c.name === seg.DE)!.views >= 12); i++) {
    const r = await fetch(`${GLOT}/api/projects/relay/xp/${xpId}/results`, { headers: { cookie } });
    assert.equal(r.status, 200);
    latest = (await r.json()).latest;
    if (!latest || latest.cells.find((c) => c.name === seg.DE)!.views < 12) await sleep(500);
  }
  assert.ok(latest, "results API: no analysis yet (rebalance worker has not ticked)");
  const de = latest.cells.find((c) => c.name === seg.DE)!;
  assert.ok(de.views >= 12, `results: ${JSON.stringify(de)}`);
  assert.ok(de.arms.slice(1).some((a) => a.mean > 0), "conversions reflected in the posterior means");
});

test("(e) glot unreachable: official text, no error, no cookie", async () => {
  for (const [path, locale] of [["/", "en"], ["/fr", "fr"], ["/de", "de"]] as const) {
    const t0 = Date.now();
    const r = await fetch(DOWN + path, { headers: { "user-agent": UA, "cf-ipcountry": "DE" } });
    assert.equal(r.status, 200);
    assert.ok(Date.now() - t0 < 3000, "never blocks on glot");
    const html = await r.text();
    assert.ok(html.includes(official[locale]["home.hero.title"]), path);
    assert.equal(r.headers.get("set-cookie"), null);
  }
  const ctx = await visitor({ country: "DE" });
  const p = await ctx.newPage();
  const errs: string[] = [];
  p.on("pageerror", (e) => errs.push(String(e)));
  p.on("console", (m) => { if (m.type() === "error" && !m.text().includes("Failed to load resource")) errs.push(m.text()); });
  await p.goto(DOWN + "/de");
  await p.waitForLoadState("networkidle");
  assert.deepEqual(await heroText(p), Object.values(official.de));
  assert.deepEqual(errs, []);
  await ctx.close();
});

test("(f) Googlebot: same treatment as a human (no cloaking), exposure not counted", async () => {
  await sleep(1000); // let earlier exposures land
  const before = total(counts("$exposure", "*")) + total(counts("$exposure", seg.DE));
  const arms = new Set<number>();
  for (let i = 0; i < 6; i++) {
    const ctx = await visitor({ ua: GOOGLEBOT, country: i % 2 ? "DE" : "US" });
    const p = await ctx.newPage();
    await p.goto(XP + "/");
    const a = await assignment(ctx); // drawn exactly like a human (weights exclude the official arm)
    arms.add(a.arm);
    assert.ok(a.arm >= 1);
    assert.equal(a.cell, i % 2 ? seg.DE : "*");
    assert.deepEqual(await heroText(p), Object.values(textOf(a.arm, "en")));
    await ctx.close();
  }
  // Same URL, same markup as a human: only the arm's texts differ.
  const strip = (h: string) => intents.concat({ intent: "", values: official }).reduce((s, x) => Object.values(x.values.en).reduce((s2, v) => s2.split(v.replace(/'/g, "&#x27;")).join("§"), s), h);
  // `accept: text/html`: a bare fetch (`*/*`) isn't a document navigation, so the edge rightly wouldn't count it.
  const human = await (await fetch(XP, { headers: { "user-agent": UA, accept: "text/html" } })).text();
  const bot = await (await fetch(XP, { headers: { "user-agent": GOOGLEBOT, accept: "text/html" } })).text();
  assert.equal(strip(bot).match(/<link rel="canonical"[^>]*>/)?.[0], strip(human).match(/<link rel="canonical"[^>]*>/)?.[0]);
  await sleep(1500);
  assert.equal(total(counts("$exposure", "*")) + total(counts("$exposure", seg.DE)), before + 1 /* the human fetch above */, "bot exposures counted");
});

test("(g) no hydration error or console error on any page visited", () => {
  assert.deepEqual(errors.filter((e) => !e.includes("Failed to load resource")), []);
});

test("(h) Lighthouse: no LCP / CLS regression vs the same app without experiments", async () => {
  const { default: lighthouse } = await import("lighthouse");
  const { default: desktop } = await import("lighthouse/core/config/desktop-config.js");
  const run = async (url: string) => {
    const r = await lighthouse(url, { port: 3499, output: "json", logLevel: "error", onlyCategories: ["performance"] }, desktop);
    const a = r!.lhr.audits;
    return { lcp: a["largest-contentful-paint"].numericValue!, cls: a["cumulative-layout-shift"].numericValue! };
  };
  const median = (xs: number[]) => xs.sort((a, b) => a - b)[Math.floor(xs.length / 2)];
  const ttfb = async (url: string) => {
    const xs: number[] = [];
    for (let i = 0; i < 15; i++) {
      const t0 = performance.now();
      const r = await fetch(url, { headers: { "user-agent": "Chrome-Lighthouse" } });
      xs.push(performance.now() - t0);
      await r.text();
    }
    return median(xs);
  };
  const [tx, tb] = [await ttfb(XP + "/de"), await ttfb(BASE + "/de")];
  console.log(`TTFB median: with experiments ${tx.toFixed(1)} ms, without ${tb.toFixed(1)} ms`);
  assert.ok(tx <= tb + 20, "the edge decision must not slow the response");
  const m: Record<string, { lcp: number[]; cls: number[] }> = { xp: { lcp: [], cls: [] }, base: { lcp: [], cls: [] } };
  for (let i = 0; i < 3; i++)
    for (const [k, url] of [["xp", XP + "/de"], ["base", BASE + "/de"]] as const) {
      const r = await run(url);
      m[k].lcp.push(r.lcp);
      m[k].cls.push(r.cls);
    }
  const xp = { lcp: median(m.xp.lcp), cls: median(m.xp.cls) }, base = { lcp: median(m.base.lcp), cls: median(m.base.cls) };
  console.log(`Lighthouse (desktop, median of 3) with experiments: LCP ${xp.lcp.toFixed(0)} ms, CLS ${xp.cls.toFixed(3)} | without: LCP ${base.lcp.toFixed(0)} ms, CLS ${base.cls.toFixed(3)}`);
  assert.ok(xp.cls <= base.cls + 0.01, "CLS regression");
  assert.ok(xp.lcp <= base.lcp * 1.2 + 100, "LCP regression");
});
