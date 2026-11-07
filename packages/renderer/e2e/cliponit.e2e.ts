// glot Edge delivery check on a real SPA: a copy of ClipOnIt (Leptos CSR + pre-rendered snapshots, Rust/axum).
//   CLIPONIT_DIST=<copy>/web/dist node e2e/cliponit.e2e.ts
// Needs: Postgres (DATABASE_URL, default :5433; a throwaway database is created and dropped), the glot server
// binary (GLOT_SERVER_BIN, default target/integ/debug/glot-server), Chromium (Playwright). Nothing leaves the box:
// the project's LLM is `mock`, so the "Spanish" values are glot's offline pseudo-translations.
// Steps: origin (same rules as ClipOnIt's src/seo.rs) → glot server (subdomain proxy, Host es.cliponit.test) →
// auto-repair (first visit discovers, the edge_translate job fills the TM, next visit is translated) →
// no-JS first byte, SEO head, browser hydration/flash (as shipped, and with the patcher tag injected),
// version coherence, kill switch, fail-open (incl. Caddy's fallback when the glot server is down, if Docker is
// available), TTFB overhead + metrics.
import { createServer, request as httpRequest, type Server } from "node:http";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, extname } from "node:path";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";
import { chromium, type Browser } from "playwright";
import { probe } from "./lib.ts";
import { ttfb, summary } from "../bench/ttfb.ts";
import { createEdge } from "../../edge/src/edge.ts";
import { segmentHashed } from "../../edge/src/segment.ts";
import { patcherTag, injectHead } from "../../edge/src/hydration.ts";
const inject = (html: string, src: string, tm: string, source: string) => injectHead(html, patcherTag(src, tm, source));

const ROOT = new URL("../../../", import.meta.url).pathname;
const DIST = process.env.CLIPONIT_DIST ?? "";
if (!DIST || !existsSync(join(DIST, "prerender/pricing.fr.html"))) {
  console.error("CLIPONIT_DIST=<copy of saasclip>/web/dist with prerender/ snapshots (node web/prerender.mjs in the copy)");
  process.exit(2);
}
const BIN = process.env.GLOT_SERVER_BIN ?? join(ROOT, "target/integ/debug/glot-server");
const PG = process.env.DATABASE_URL ?? "postgres://glot:glot@localhost:5433/glot";
const PAGES = process.env.E2E_PAGES?.split(",") ?? ["/", "/pricing", "/explainer-video", "/cliping"];
const ROUTE: Record<string, string> = { "/": "home", "/pricing": "pricing", "/explainer-video": "explainer-video", "/cliping": "cliping" };
const HOST = "es.cliponit.test";
const results: { name: string; ok: boolean; detail: string }[] = [];
const check = (name: string, ok: boolean, detail = "") => { results.push({ name, ok, detail }); console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  — ${detail}` : ""}`); };
const PATCHER = join(ROOT, "web/public/widget/edge-patcher.global.js");
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const listen = (s: Server) => new Promise<number>((r) => s.listen(0, "127.0.0.1", () => r((s.address() as { port: number }).port)));

// ---------------------------------------------------------------- origin: ClipOnIt's src/seo.rs rules
const MIME: Record<string, string> = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".mp4": "video/mp4", ".txt": "text/plain; charset=utf-8", ".ico": "image/x-icon" };
const langFor = (q: URLSearchParams, accept: string) => (q.get("lang") ?? accept.split(",").map((t) => t.trim().toLowerCase()).find((t) => t.startsWith("fr") || t.startsWith("en"))?.slice(0, 2)) === "en" ? "en" : "fr";
let originCacheControl = "no-cache";
let originNonce = false; // a changing comment per response: the body is never the same (translate every time)
const origin = createServer((req, res) => {
  const u = new URL(req.url ?? "/", "http://o");
  // glot's patcher bundle (GLOT_EDGE_PATCHER_SRC), what the dashboard host serves in production
  if (u.pathname === "/__e2e/edge-patcher.global.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(readFileSync(PATCHER));
  if (u.pathname === "/api/billing/plans") return res.writeHead(200, { "content-type": "application/json" }).end(readFileSync(join(DIST, "../plans.json")));
  const route = ROUTE[u.pathname];
  if (route) {
    const lang = langFor(u.searchParams, req.headers["accept-language"] ?? "");
    return res.writeHead(200, { "content-type": MIME[".html"], "content-language": lang, vary: "Accept-Language", "cache-control": originCacheControl, "x-frame-options": "SAMEORIGIN" }).end(readFileSync(join(DIST, `prerender/${route}.${lang}.html`), "utf8") + (originNonce ? `<!-- ${randomUUID()} -->` : ""));
  }
  const p = join(DIST, decodeURIComponent(u.pathname));
  if (u.pathname.startsWith("/api/")) return res.writeHead(404, { "content-type": "application/json" }).end("{}");
  if (existsSync(p) && statSync(p).isFile()) return res.writeHead(200, { "content-type": MIME[extname(p)] ?? "application/octet-stream", "cache-control": "public, max-age=3600" }).end(readFileSync(p));
  res.writeHead(404, { "content-type": MIME[".html"], "x-robots-tag": "noindex" }).end(readFileSync(join(DIST, "index.html")));
});
const ORIGIN = `http://127.0.0.1:${await listen(origin)}`;

// ---------------------------------------------------------------- glot server on a throwaway database
const db = `glot_e2e_edge_${randomUUID().replace(/-/g, "").slice(0, 12)}`;
const psql = (sql: string, database = db) => execFileSync("psql", [PG.replace(/\/[^/]*$/, `/${database}`), "-qtAX", "-v", "ON_ERROR_STOP=1", "-c", sql], { encoding: "utf8" }).trim();
psql(`CREATE DATABASE ${db}`, "glot");
const GLOT_PORT = Number(process.env.GLOT_E2E_PORT ?? 8797);
const GLOT = `http://127.0.0.1:${GLOT_PORT}`;
const cacheDir = mkdtempSync(join(tmpdir(), "glot-edge-e2e-"));
const caddyDir = mkdtempSync(join(tmpdir(), "glot-edge-caddy-"));
let server: ChildProcess | undefined;
async function startServer() {
  server = spawn(BIN, [], { env: { ...process.env, DATABASE_URL: PG.replace(/\/[^/]*$/, `/${db}`), GLOT_BIND: `127.0.0.1:${GLOT_PORT}`, GLOT_EDGE_ALLOW_PRIVATE: "1", GLOT_EDGE_CACHE_DIR: cacheDir, GLOT_SECRET_KEY: "e2e", RUST_LOG: "warn", GLOT_EDGE_PATCHER_SRC: `${ORIGIN}/__e2e/edge-patcher.global.js`, GLOT_CADDY_MAP: join(caddyDir, "origins.caddy") }, stdio: ["ignore", "ignore", "inherit"] });
  for (let i = 0; i < 200; i++) { if (await fetch(`${GLOT}/healthz`).then((r) => r.ok, () => false)) return; await sleep(100); }
  throw new Error("glot server did not start");
}
const stopServer = async () => { server?.kill("SIGTERM"); await new Promise((r) => server?.once("exit", r)); server = undefined; };
let browser: Browser | undefined;
let caddy = "";
async function cleanup() {
  if (caddy) try { execFileSync("docker", ["rm", "-f", caddy], { stdio: "ignore" }); } catch { /* best effort */ }
  await browser?.close().catch(() => {});
  await stopServer().catch(() => {});
  origin.close();
  try { psql(`DROP DATABASE IF EXISTS ${db} WITH (FORCE)`, "glot"); } catch { /* best effort */ }
}
process.on("SIGINT", () => void cleanup().then(() => process.exit(130)));
process.on("exit", () => server?.kill("SIGKILL"));

/** GET with a Host header (fetch/undici drops it): what Caddy sends to the glot server. */
function hget(url: string, headers: Record<string, string>, method = "GET", payload?: Buffer): Promise<{ status: number; headers: Headers; text: () => Promise<string>; body: Buffer }> {
  return new Promise((resolve, reject) => {
    const r = httpRequest(url, { headers, method }, (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const h = new Headers();
        for (const [k, v] of Object.entries(res.headers)) if (v !== undefined) h.set(k, Array.isArray(v) ? v.join(", ") : v);
        const body = Buffer.concat(chunks);
        resolve({ status: res.statusCode ?? 0, headers: h, body, text: async () => body.toString("utf8") });
      });
    });
    r.on("error", reject);
    r.end(payload);
  });
}
const edge = (path: string, h: Record<string, string> = {}) => hget(`${GLOT}/edge/proxy${path}`, { host: HOST, accept: "text/html", "accept-language": "es-ES,es;q=0.9", "accept-encoding": "identity", ...h });
const title = (h: string) => /<title[^>]*>([^<]*)<\/title>/.exec(h)?.[1] ?? "";
const h1 = (h: string) => (/<h1[^>]*>([\s\S]*?)<\/h1>/.exec(h)?.[1] ?? "").replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();

try {
  await startServer();
  const project = randomUUID();
  psql(`INSERT INTO projects (id, slug, name, source_locale, target_locales, public_cdn, llm) VALUES ('${project}', 'cliponit', 'ClipOnIt (copy)', 'fr', '{es}', true, '{"provider": "mock", "model": "mock"}')`);
  psql(`INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at) VALUES ('${randomUUID()}', '${project}', '${HOST}', 'subdomain', 'es', '${ORIGIN}', 't', now())`);
  // The domain was inserted behind glot's back: a restart rewrites Caddy's fallback map (startup sync).
  await stopServer();
  await startServer();

  // ------------------------------------------------------------ 1. auto-repair: discover → translate → serve
  const first = await edge("/pricing");
  const firstHtml = await first.text();
  check("first visit, empty TM: source served, never a hole or an error", first.status === 200 && h1(firstHtml).length > 0, `status ${first.status}, x-glot-edge=${first.headers.get("x-glot-edge")}, h1="${h1(firstHtml)}"`);
  for (const p of PAGES) await edge(p).then((r) => r.text());
  let coverage: Record<string, string> = {};
  for (let i = 0; i < 120; i++) {
    coverage = {};
    for (const p of PAGES) coverage[p] = (await edge(p).then(async (r) => (await r.text(), r.headers.get("x-glot-coverage")))) ?? "?";
    if (Object.values(coverage).every((c) => /^(\d+)\/\1$/.test(c))) break;
    await sleep(1000);
  }
  const full = Object.values(coverage).every((c) => /^(\d+)\/\1$/.test(c));
  check("auto-repair: every segment of the 4 pages translated after discovery (no key, no code change)", full, JSON.stringify(coverage));
  if (!full) {
    const jobs = psql(`SELECT kind || ':' || status || ':' || coalesce(left(error, 160), '') FROM jobs ORDER BY created_at DESC LIMIT 3`);
    const n = psql(`SELECT count(*) FROM edge_segments`) + " segments / " + psql(`SELECT count(*) FROM edge_translations`) + " translations";
    console.log(`  jobs: ${jobs.replace(/\n/g, " | ")}  db: ${n}`);
  }

  // A locale is advertised (hreflang, canonical) once ready; the proxy re-reads it within its domain TTL (15 s).
  for (let i = 0; i < 40; i++) {
    if ((await edge("/pricing").then((r) => r.text())).includes('rel="canonical"')) break;
    await sleep(1000);
  }

  // ------------------------------------------------------------ 2. no-JS first byte + SEO head
  const tmVersion = Number(psql(`SELECT version FROM edge_catalog_version WHERE project_id = '${project}' AND locale = 'es'`) || 0);
  const sources: Record<string, string[]> = {};
  for (const p of PAGES) {
    const fr = readFileSync(join(DIST, `prerender/${ROUTE[p]}.fr.html`), "utf8");
    const r = await edge(p);
    const html = await r.text();
    const lang = /<html[^>]*\blang="([^"]*)"/.exec(html)?.[1];
    const canon = /<link[^>]*rel="canonical"[^>]*href="([^"]*)"|<link[^>]*href="([^"]*)"[^>]*rel="canonical"/.exec(html);
    const alts = [...html.matchAll(/<link[^>]*hreflang="([^"]*)"[^>]*>/g)].map((m) => `${m[1]}=${/href="([^"]*)"/.exec(m[0])?.[1]}`);
    const ok = r.status === 200 && lang === "es" && r.headers.get("content-language") === "es" && title(html) !== title(fr) && h1(html) !== h1(fr) && h1(html).length > 0;
    check(`no-JS first byte translated ${p}`, ok, `lang=${lang} content-language=${r.headers.get("content-language")} title="${title(html).slice(0, 60)}" h1="${h1(html).slice(0, 60)}" (fr h1="${h1(fr).slice(0, 40)}") x-glot-version=${r.headers.get("x-glot-version")}`);
    const canonHref = canon?.[1] ?? canon?.[2] ?? "";
    const selfUrl = `https://${HOST}${p}`;
    const hasEs = alts.some((a) => a === `es=${selfUrl}` || a.startsWith("es=") && a.endsWith(p === "/" ? "/" : p));
    const hasFr = alts.some((a) => a.startsWith("fr="));
    const hasXd = alts.some((a) => a.startsWith("x-default="));
    check(`hreflang/canonical ${p}`, canonHref.includes(HOST) && hasEs && hasFr && hasXd, `canonical=${canonHref} alternates=${alts.join(" ")}`);
    // French strings of this page that must never be visible on the Spanish page
    // (every plain-text segment: single text node, so "visible" = a text node equal to it)
    const all = await segmentHashed(fr, { sourceLocale: "fr" });
    // With the offline mock ("⟦es⟧ " + source), a text node of a placeholder segment (`<0/>Accès anticipé`)
    // legitimately keeps its source words after the prefix: such texts are not counted.
    const inTags = all.filter((s) => /[<{]/.test(s.text)).map((s) => s.text);
    const segs = all.filter((s) => !s.attr && !/[<{]/.test(s.text) && /\p{L}{3}/u.test(s.text) && !inTags.some((t) => t.includes(s.text)));
    sources[p] = [...new Set(segs.map((s) => s.text))];
  }

  // ------------------------------------------------------------ 3. browser: hydration, flash, head (as served by glot)
  browser = await chromium.launch();
  // Front proxy = what Caddy does in production (Host kept, /edge/proxy prefix). The page is exactly what the glot
  // proxy serves: translated HTML + its same-origin patcher (`/__glot/patcher.js`, `/__glot/tm.json`,
  // `/__glot/missing` for text the app renders on the client only).
  const front = createServer(async (req, res) => {
    const chunks: Buffer[] = [];
    for await (const c of req) chunks.push(c as Buffer);
    const fwd: Record<string, string> = { host: HOST, accept: req.headers.accept ?? "*/*", "accept-language": req.headers["accept-language"] ?? "", "accept-encoding": "identity" };
    if (req.headers["content-type"]) fwd["content-type"] = req.headers["content-type"];
    const up = await hget(`${GLOT}/edge/proxy${req.url}`, fwd, req.method ?? "GET", chunks.length ? Buffer.concat(chunks) : undefined);
    const headers: Record<string, string> = {};
    for (const h of ["content-type", "x-glot-edge", "x-glot-version", "content-language", "server-timing", "cache-control"]) { const v = up.headers.get(h); if (v) headers[h] = v; }
    res.writeHead(up.status, headers).end(up.body);
  });
  const FRONT = `http://127.0.0.1:${await listen(front)}`;
  const boot = async (page: import("playwright").Page) => {
    // ClipOnIt starts its wasm on the first intent (pointerdown/focus on a control of the snapshot) or 5 s idle
    await page.dispatchEvent("#prerender a, #prerender button", "focusin").catch(() => {});
    await page.waitForFunction(() => !document.getElementById("prerender"), null, { timeout: 15000 }).catch(() => {});
  };
  // Auto-repair of client-only text: a first browser visit per page lets the patcher report what the app renders
  // after boot (never in the server HTML); glot translates it, then the measured visits below must show no source.
  const untranslated = () => Number(psql(`SELECT count(*) FROM edge_segments s WHERE s.project_id = '${project}' AND NOT EXISTS (SELECT 1 FROM edge_translations t WHERE t.project_id = s.project_id AND t.hash = s.hash AND t.context = s.context AND t.locale = 'es')`));
  const before = Number(psql(`SELECT count(*) FROM edge_segments WHERE project_id = '${project}'`));
  for (const p of PAGES) await probe(browser, FRONT + p, { sources: [], targets: [], settle: 3000, act: boot, locale: "es-ES" });
  let left = untranslated();
  for (let i = 0; i < 90 && left > 0; i++) { await sleep(1000); left = untranslated(); }
  const after = Number(psql(`SELECT count(*) FROM edge_segments WHERE project_id = '${project}'`));
  check("client-only text discovered by the patcher and translated", left === 0, `${after - before} new segment(s) reported from the browser, ${left} still untranslated`);
  await sleep(3500); // > VERSION_TTL: the proxy serves the new TM version
  // baseline: the same page straight from the origin (the app's own console noise is not glot's)
  for (const p of PAGES) {
    const base = await probe(browser, ORIGIN + p, { sources: [], targets: [], settle: 2500, act: boot, locale: "es-ES" });
    const r = await probe(browser, FRONT + p, { sources: sources[p], targets: [], settle: 2500, act: boot, locale: "es-ES" });
    const extra = r.errors.filter((e) => !base.errors.includes(e));
    if (r.flashes.length && process.env.DEBUG) {
      const tm = await fetch(`${GLOT}/edge/v1/cliponit/es/tm`).then((x) => x.json() as Promise<{ t: Record<string, unknown> }>);
      for (const html of r.samples) {
        for (const sg of await segmentHashed(html, { sourceLocale: "fr" })) console.log(`    seg role=${sg.role} in TM=${sg.hash in tm.t} ${JSON.stringify(tm.t[sg.hash] ?? null).slice(0, 120)} text=${JSON.stringify(sg.text).slice(0, 80)}`);
        console.log(`    block: ${html.slice(0, 300)}`);
      }
    }
    check(`browser ${p} as served by glot (proxy-injected patcher)`, !extra.length && !r.flashes.length && r.mounted !== false && r.lang === "es", `app mounted=${r.mounted} new console errors=${extra.length}${extra.length ? ` ${JSON.stringify(extra.slice(0, 2))}` : ""} French visible after boot=${r.flashes.length}${r.flashes.length ? ` e.g. ${JSON.stringify(r.flashes.slice(0, 3))}` : ""} html lang=${r.lang} canonical=${r.canonical}`);
  }
  // no JS at all: what a crawler without rendering gets
  {
    const ctx = await browser.newContext({ javaScriptEnabled: false, locale: "es-ES" });
    const pg = await ctx.newPage();
    await pg.goto(FRONT + "/pricing");
    const t = await pg.evaluate(() => document.querySelector("h1")?.textContent ?? "");
    check("JavaScript disabled: translated content readable", t.trim().startsWith("⟦es⟧"), `h1="${t.trim().slice(0, 60)}"`);
    await ctx.close();
  }

  // ------------------------------------------------------------ 4. one version everywhere
  {
    const seg = psql(`SELECT hash || '|' || context FROM edge_translations WHERE project_id = '${project}' AND locale = 'es' AND context LIKE 'h1|/pricing' LIMIT 1`);
    const [hash, ctx] = seg.split("|").length > 2 ? [seg.split("|")[0], seg.split("|").slice(1).join("|")] : seg.split("|");
    const v0 = Number(psql(`SELECT version FROM edge_catalog_version WHERE project_id = '${project}' AND locale = 'es'`));
    psql(`UPDATE edge_translations SET value = 'Precios EDITADOS v2', origin = 'human', status = 'reviewed' WHERE project_id = '${project}' AND locale = 'es' AND hash = '${hash}' AND context = '${ctx}'`);
    const v1 = Number(psql(`SELECT version FROM edge_catalog_version WHERE project_id = '${project}' AND locale = 'es'`));
    let html = "", ver = "", tmv = 0, tmHas = false;
    for (let i = 0; i < 40; i++) {
      const r = await edge("/pricing");
      html = await r.text();
      ver = r.headers.get("x-glot-version") ?? "";
      const tm = await fetch(`${GLOT}/edge/v1/cliponit/es/tm`).then((x) => x.json() as Promise<{ v: number; t: Record<string, unknown> }>);
      tmv = tm.v;
      tmHas = JSON.stringify(tm.t[hash] ?? "").includes("EDITADOS");
      if (html.includes("Precios EDITADOS v2") && tmHas) break;
      await sleep(500);
    }
    const meta = /<meta name="glot-tm" content="([^"]*)"/.exec(html)?.[1] ?? "(no glot-tm meta)";
    check("edit in glot → proxy and TM endpoint serve the new value under one new version", v1 > v0 && html.includes("Precios EDITADOS v2") && tmHas && ver === `es@${v1}` && tmv === v1, `version ${v0}→${v1}, proxy x-glot-version=${ver} meta=${meta}, TM endpoint v=${tmv} has edit=${tmHas}, h1="${h1(html)}"`);
    void tmVersion;
  }

  // ------------------------------------------------------------ 5. TTFB overhead, cache, metrics
  {
    const N = Number(process.env.BENCH_N ?? 30);
    const direct = await ttfb(`${ORIGIN}/pricing`, {}, N);
    originNonce = true; // body differs every time: a full translate pass per request
    const cold = await ttfb(`${GLOT}/edge/proxy/pricing`, { host: HOST, accept: "text/html" }, N);
    originNonce = false; // ClipOnIt as is: `no-cache`, same body → revalidated, translation reused
    await edge("/pricing").then((r) => r.text());
    const miss = await ttfb(`${GLOT}/edge/proxy/pricing`, { host: HOST, accept: "text/html" }, N);
    originCacheControl = "public, max-age=300"; // same pages, now cacheable by the origin's own rules
    await edge("/pricing").then((r) => r.text());
    const hit = await ttfb(`${GLOT}/edge/proxy/pricing`, { host: HOST, accept: "text/html" }, N);
    originCacheControl = "no-cache";
    const d = summary(direct), c = summary(cold), m = summary(miss), h = summary(hit);
    console.log(`  TTFB /pricing (n=${N}, loopback origin): origin p50 ${d.p50} ms p95 ${d.p95} | edge, body changes every time p50 ${c.p50} p95 ${c.p95} [${c.states}] | edge, origin no-cache, same body p50 ${m.p50} p95 ${m.p95} [${m.states}] | edge page cache p50 ${h.p50} p95 ${h.p95} [${h.states}]`);
    check("TTFB overhead measured", c.p50 !== null && m.p50 !== null && h.p50 !== null, `translate pass +${(c.p50! - d.p50!).toFixed(1)} ms p50; no-cache origin, unchanged body +${(m.p50! - d.p50!).toFixed(1)} ms p50 / +${(m.p95! - d.p95!).toFixed(1)} ms p95; page cache +${(h.p50! - d.p50!).toFixed(1)} ms p50; glot share (Server-Timing glot;dur) p50 ${c.glotP50} → ${m.glotP50} ms`);
    check("no-cache origin: revalidated every request, unchanged body's translation reused", m.states.includes("cache=hit-body"), `states [${m.states}]`);
  }

  // ------------------------------------------------------------ 6. Caddy in front (Docker), kill switch, fail-open
  // deploy/Caddyfile's edge site block on plain HTTP, glot = the host's server, the map glot wrote at startup
  // (origin address rewritten to the host as seen from the container).
  let caddyPort = 0;
  try {
    execFileSync("docker", ["image", "inspect", "caddy:2"], { stdio: "ignore" });
    const conf = readFileSync(join(ROOT, "deploy/Caddyfile"), "utf8");
    const block = conf.slice(conf.indexOf("https:// {")).replace("https:// {", "http://:8080 {").replace(/\ttls \{\n\t\ton_demand\n\t\}\n/, "").replace("reverse_proxy server:8790", `reverse_proxy host.docker.internal:${GLOT_PORT}`);
    writeFileSync(join(caddyDir, "Caddyfile"), `{\n\tadmin off\n}\n${block}`);
    mkdirSync(join(caddyDir, "edge"), { recursive: true });
    const map = readFileSync(join(caddyDir, "origins.caddy"), "utf8");
    writeFileSync(join(caddyDir, "edge/origins.caddy"), map.replaceAll("127.0.0.1:", "host.docker.internal:"));
    caddy = execFileSync("docker", ["run", "-d", "--rm", "-p", "127.0.0.1::8080", "-v", `${caddyDir}/Caddyfile:/etc/caddy/Caddyfile:ro`, "-v", `${caddyDir}/edge:/etc/caddy/edge:ro`, "caddy:2"], { encoding: "utf8" }).trim();
    caddyPort = Number(execFileSync("docker", ["port", caddy, "8080/tcp"], { encoding: "utf8" }).trim().split(":").pop());
    let r: Awaited<ReturnType<typeof hget>> | undefined;
    for (let i = 0; i < 50 && !r; i++) { r = await hget(`http://127.0.0.1:${caddyPort}/pricing`, { host: HOST, accept: "text/html" }).catch(() => undefined); if (!r) await sleep(200); }
    check("glot wrote Caddy's fallback map; through Caddy (glot up) the page is translated", map.includes(`${HOST} 127.0.0.1:`) && r?.status === 200 && (r.headers.get("x-glot-edge") ?? "").startsWith("translated"), `map line: ${map.split("\n").find((l) => l.includes(HOST))?.trim()} | via Caddy: ${r?.status} x-glot-edge=${r?.headers.get("x-glot-edge")}`);
  } catch (e) {
    console.log(`  (Caddy fallback not exercised: ${String(e).split("\n")[0]})`);
  }

  {
    psql(`UPDATE edge_domains SET enabled = false WHERE host = '${HOST}'`);
    const t0 = Date.now();
    let state = "";
    while (Date.now() - t0 < 60_000) { const r = await edge("/pricing"); await r.text(); state = r.headers.get("x-glot-edge") ?? ""; if (state === "off" || state === "passthrough") break; await sleep(1000); }
    check("kill switch (domain disabled) → origin page in < 1 min", state === "off" || state === "passthrough", `x-glot-edge=${state} after ${((Date.now() - t0) / 1000).toFixed(1)} s`);
    psql(`UPDATE edge_domains SET enabled = true WHERE host = '${HOST}'`);
    for (let i = 0; i < 60; i++) { const r = await edge("/pricing"); await r.text(); if (r.headers.get("x-glot-edge")?.startsWith("translated")) break; await sleep(1000); }
  }
  {
    // Postgres unreachable for glot (connections to this database refused and killed): last known state
    psql(`ALTER DATABASE ${db} WITH ALLOW_CONNECTIONS false`, "glot");
    psql(`SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${db}'`, "glot");
    await sleep(500);
    const t0 = performance.now();
    const r = await edge("/pricing");
    const html = await r.text();
    const ms = performance.now() - t0;
    check("glot's database down → page still served translated from the last known state, no 5xx", r.status === 200 && (r.headers.get("x-glot-edge") ?? "").startsWith("translated") && h1(html) === "Precios EDITADOS v2", `status ${r.status} x-glot-edge=${r.headers.get("x-glot-edge")} in ${ms.toFixed(0)} ms, h1="${h1(html).slice(0, 50)}"`);
    psql(`ALTER DATABASE ${db} WITH ALLOW_CONNECTIONS true`, "glot");
  }
  {
    // glot server gone entirely, in the modes where the origin is in front (Worker / Next / Express runtime)
    await stopServer();
    const js = createEdge({ server: GLOT, project: "cliponit", source: "fr", translate: () => { throw new Error("unreachable"); }, timeout: 300 });
    const t0 = performance.now();
    const r = await js(new Request(`https://cliponit.test/es/pricing`, { headers: { accept: "text/html" } }), "es", new URL(`${ORIGIN}/pricing`), (u) => fetch(u));
    const html = await r.text();
    const ms = performance.now() - t0;
    const fr = readFileSync(join(DIST, "prerender/pricing.fr.html"), "utf8");
    check("glot server down → origin page untranslated (fail-open), no error", r.status === 200 && r.headers.get("x-glot-edge") === "fail-open" && h1(html) === h1(fr), `status ${r.status} x-glot-edge=${r.headers.get("x-glot-edge")} in ${ms.toFixed(0)} ms (budget 300 ms)`);
    if (caddyPort) {
      // Caddy in front (deploy/Caddyfile edge block): the glot server is gone, the origin is served from the map
      const r = await hget(`http://127.0.0.1:${caddyPort}/pricing`, { host: HOST, accept: "text/html" }).catch(() => undefined);
      const html = (await r?.text()) ?? "";
      check("subdomain mode, glot server process down → Caddy serves the origin (no 502)", r?.status === 200 && (r.headers.get("x-glot-edge") ?? "").startsWith("fail-open; glot=down") && h1(html) === h1(fr), `status ${r?.status} x-glot-edge=${r?.headers.get("x-glot-edge")} h1="${h1(html).slice(0, 50)}"`);
    }
    await startServer();
  }
  check("server back up after the outage", await fetch(`${GLOT}/healthz`).then((r) => r.ok, () => false));
} catch (e) {
  check("e2e run", false, String((e as Error)?.stack ?? e));
} finally {
  await cleanup();
}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} passed${failed.length ? `; failed: ${failed.map((f) => f.name).join(" | ")}` : ""}`);
process.exit(failed.length ? 1 : 0);
