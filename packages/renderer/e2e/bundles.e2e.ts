// Bundle localization, end to end: text hard-coded in client JavaScript (a Next.js client component, a Pages
// Router page, a Vite + React SSR app) hydrates on the translated HTML without React #418 and without a flash,
// because the glot proxy also serves the chunks with those strings translated (crates/server/src/edge/bundle.rs).
//   NEXT_URL=http://127.0.0.1:4621 VITE_DIR=<built fixtures/vite-react> node e2e/bundles.e2e.ts
// Fixtures: e2e/fixtures/next-app (`next build && next start -p 4621`), e2e/fixtures/vite-react
// (`vite build --outDir dist/client && vite build --ssr src/entry-server.jsx --outDir dist/server`).
// Needs Postgres (DATABASE_URL, default :5433; a throwaway database is created and dropped), the glot server binary
// (GLOT_SERVER_BIN, default target/integ/debug/glot-server), Chromium (Playwright). Nothing leaves the box.
// Each page is loaded twice through the real glot proxy: "baseline" (JavaScript fetched straight from the origin,
// i.e. the previous behavior) and "bundles" (everything through glot). Exit 1 when "bundles" is not clean.
import { createServer, request as httpRequest, type Server } from "node:http";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { join, extname } from "node:path";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";
import { chromium } from "playwright";
import { probe } from "./lib.ts";
import { hash16 } from "../../edge/src/segment.ts";

const ROOT = new URL("../../../", import.meta.url).pathname;
const NEXT = process.env.NEXT_URL ?? "http://127.0.0.1:4621";
const VITE_DIR = process.env.VITE_DIR ?? "";
if (!VITE_DIR || !existsSync(join(VITE_DIR, "dist/server/entry-server.js"))) {
  console.error("VITE_DIR=<built copy of e2e/fixtures/vite-react> (see header)");
  process.exit(2);
}
const BIN = process.env.GLOT_SERVER_BIN ?? join(ROOT, "target/integ/debug/glot-server");
const PG = process.env.DATABASE_URL ?? "postgres://glot:glot@localhost:5433/glot";
const PATCHER = join(ROOT, "web/public/widget/edge-patcher.global.js");
const ES: Record<string, string> = {
  "Simple pricing for teams": "Precios simples para equipos",
  "Start for free today.": "Empieza gratis hoy.",
  "Add to cart": "Añadir al carrito",
  "Cancel anytime": "Cancela cuando quieras",
  "Legacy pricing page": "Página de precios clásica",
};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const listen = (s: Server) => new Promise<number>((r) => s.listen(0, "127.0.0.1", () => r((s.address() as { port: number }).port)));
const results: boolean[] = [];
const check = (name: string, ok: boolean, detail = "") => { results.push(ok); console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  — ${detail}` : ""}`); };

// ---------------------------------------------------------------- Vite + React SSR origin (immutable assets, like `vite preview`)
const { render } = (await import(join(VITE_DIR, "dist/server/entry-server.js"))) as { render: () => string };
const template = readFileSync(join(VITE_DIR, "dist/client/index.html"), "utf8");
const MIME: Record<string, string> = { ".js": "text/javascript", ".css": "text/css" };
const vite = createServer((req, res) => {
  const u = new URL(req.url ?? "/", "http://o");
  if (u.pathname === "/__e2e/patcher.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(readFileSync(PATCHER));
  if (u.pathname.startsWith("/assets/")) {
    const p = join(VITE_DIR, "dist/client", u.pathname);
    if (!existsSync(p)) return res.writeHead(404).end();
    return res.writeHead(200, { "content-type": MIME[extname(p)] ?? "application/octet-stream", "cache-control": "public, max-age=31536000, immutable" }).end(readFileSync(p));
  }
  res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(template.replace("<!--app-->", render()));
});
const VITE = `http://127.0.0.1:${await listen(vite)}`;

// ---------------------------------------------------------------- glot server on a throwaway database
const db = `glot_e2e_bundles_${randomUUID().replace(/-/g, "").slice(0, 12)}`;
const psql = (sql: string, database = db) => execFileSync("psql", [PG.replace(/\/[^/]*$/, `/${database}`), "-qtAX", "-v", "ON_ERROR_STOP=1", "-c", sql], { encoding: "utf8" }).trim();
psql(`CREATE DATABASE ${db}`, "glot");
const GLOT_PORT = Number(process.env.GLOT_E2E_PORT ?? 8798);
const GLOT = `http://127.0.0.1:${GLOT_PORT}`;
let server: ChildProcess | undefined;
async function startServer() {
  server = spawn(BIN, [], { env: { ...process.env, DATABASE_URL: PG.replace(/\/[^/]*$/, `/${db}`), GLOT_BIND: `127.0.0.1:${GLOT_PORT}`, GLOT_EDGE_ALLOW_PRIVATE: "1", GLOT_EDGE_CACHE_DIR: mkdtempSync(join(tmpdir(), "glot-bundles-")), GLOT_SECRET_KEY: "e2e", RUST_LOG: "warn", GLOT_EDGE_PATCHER_SRC: `${VITE}/__e2e/patcher.js`, GLOT_CADDY_MAP: join(mkdtempSync(join(tmpdir(), "glot-bundles-caddy-")), "origins.caddy") }, stdio: ["ignore", "ignore", "inherit"] });
  for (let i = 0; i < 200; i++) { if (await fetch(`${GLOT}/healthz`).then((r) => r.ok, () => false)) return; await sleep(100); }
  throw new Error("glot server did not start");
}
let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
const fronts: Server[] = [];
async function cleanup() {
  await browser?.close().catch(() => {});
  server?.kill("SIGTERM");
  await new Promise((r) => (server ? server.once("exit", r) : r(null)));
  vite.close();
  for (const f of fronts) f.close();
  try { psql(`DROP DATABASE IF EXISTS ${db} WITH (FORCE)`, "glot"); } catch { /* best effort */ }
}
process.on("exit", () => server?.kill("SIGKILL"));

function hget(url: string, headers: Record<string, string>): Promise<{ status: number; headers: Headers; body: Buffer }> {
  return new Promise((resolve, reject) => {
    const r = httpRequest(url, { headers }, (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const h = new Headers();
        for (const [k, v] of Object.entries(res.headers)) if (v !== undefined) h.set(k, Array.isArray(v) ? v.join(", ") : v);
        resolve({ status: res.statusCode ?? 0, headers: h, body: Buffer.concat(chunks) });
      });
    });
    r.on("error", reject);
    r.end();
  });
}
const viaGlot = (host: string, path: string, h: Record<string, string> = {}) => hget(`${GLOT}/edge/proxy${path}`, { host, "accept-encoding": "identity", ...h });

/** What Caddy does in production (Host kept, /edge/proxy prefix). `baseline`: JS straight from the origin. */
let baseline = false;
async function front(host: string, origin: string) {
  const s = createServer(async (req, res) => {
    const js = /\.m?js(\?|$)/.test(req.url ?? "") && !req.url?.startsWith("/__glot/");
    const up = baseline && js
      ? await hget(origin + req.url, { accept: "*/*", "accept-encoding": "identity" })
      : await viaGlot(host, req.url ?? "/", { accept: req.headers.accept ?? "*/*", "accept-language": "es-ES,es;q=0.9", ...(req.headers["if-none-match"] ? { "if-none-match": String(req.headers["if-none-match"]) } : {}) });
    const headers: Record<string, string> = {};
    for (const h of ["content-type", "cache-control", "etag", "x-glot-edge", "x-glot-bundle", "x-glot-version", "content-language"]) { const v = up.headers.get(h); if (v) headers[h] = v; }
    res.writeHead(up.status, headers).end(up.body);
  });
  fronts.push(s);
  return `http://127.0.0.1:${await listen(s)}`;
}

let failed = false;
try {
  await startServer();
  const project = randomUUID();
  psql(`INSERT INTO projects (id, slug, name, source_locale, target_locales, public_cdn, llm) VALUES ('${project}', 'bundles', 'Bundles e2e', 'en', '{es}', true, '{"provider": "mock", "model": "mock"}')`);
  for (const [host, origin] of [["es.next.test", NEXT], ["es.vite.test", VITE]])
    psql(`INSERT INTO edge_domains (id, project_id, host, mode, locale, origin_url, verify_token, verified_at) VALUES ('${randomUUID()}', '${project}', '${host}', 'subdomain', 'es', '${origin}', 't', now())`);
  // The TM: segments as the proxy discovers them from rendered pages (context = role|template), reviewed values.
  const rows = Object.entries(ES).map(([s]) => `('${project}', '${hash16(`en\u0000${s}`)}', 'p|/', '${s}')`).join(",");
  psql(`INSERT INTO edge_segments (project_id, hash, context, source_text) VALUES ${rows}`);
  const tr = Object.entries(ES).map(([s, t]) => `('${project}', '${hash16(`en\u0000${s}`)}', 'p|/', 'es', '${t}', 'reviewed', 'human')`).join(",");
  psql(`INSERT INTO edge_translations (project_id, hash, context, locale, value, status, origin) VALUES ${tr}`);

  // ------------------------------------------------------------ 1. the chunk itself (headers, cache, version)
  const html = (await viaGlot("es.next.test", "/app-mixed", { accept: "text/html" })).body.toString();
  // the chunk holding the client component (webpack: app/app-mixed/page-*.js; Turbopack: a hashed name)
  const srcs = [...new Set([...html.matchAll(/(?:src|href)="(\/_next\/static\/chunks\/[^"]+\.js)"/g)].map((m) => m[1]!))];
  let chunk = "";
  let c1 = await viaGlot("es.next.test", srcs[0] ?? "/");
  for (const c of srcs) {
    const r = await viaGlot("es.next.test", c);
    if (r.body.includes("Cancela cuando quieras")) { chunk = c; c1 = r; break; }
  }
  const etag = c1.headers.get("etag") ?? "";
  check("Next client chunk served localized (string literal translated, keys and code untouched)", c1.body.includes('"Cancela cuando quieras"') && !c1.body.includes('"Cancel anytime"') && Number(c1.headers.get("x-glot-bundle")) >= 1, `chunk=${chunk} x-glot-bundle=${c1.headers.get("x-glot-bundle")} x-glot-edge=${c1.headers.get("x-glot-edge")}`);
  check("immutable dropped: public, no-cache + strong ETag", c1.headers.get("cache-control") === "public, no-cache" && etag.startsWith('"glot-'), `cache-control=${c1.headers.get("cache-control")} etag=${etag}`);
  const c304 = await viaGlot("es.next.test", chunk, { "if-none-match": etag });
  check("revalidation with the same version: 304, no body", c304.status === 304 && c304.body.length === 0, `status ${c304.status}`);
  const fw = srcs.find((c) => c !== chunk);
  if (fw) {
    const f = await viaGlot("es.next.test", fw);
    check(`chunk without matching text (${fw}): unchanged bytes`, f.headers.get("x-glot-bundle") === "0" && f.body.equals((await hget(NEXT + fw, {})).body), `x-glot-bundle=${f.headers.get("x-glot-bundle")}`);
  }

  // Worker / middleware mode (one URL for every language): the chunk body posted to glot with the public key.
  psql(`INSERT INTO insight_keys (project_id, public_key) VALUES ('${project}', 'pk_bundles')`);
  const src = new Uint8Array((await hget(NEXT + chunk, {})).body);
  const post = (key: string) => fetch(`${GLOT}/edge/v1/bundles/es/bundle`, { method: "POST", headers: { "content-type": "text/javascript", "x-glot-key": key }, body: src });
  const w = await post("pk_bundles");
  const wBody = await w.text();
  check("worker endpoint: chunk localized by glot (POST …/es/bundle, public key)", w.status === 200 && wBody.includes('"Cancela cuando quieras"') && w.headers.get("etag") === etag, `status ${w.status} x-glot-bundle=${w.headers.get("x-glot-bundle")} etag=${w.headers.get("etag")}`);
  check("worker endpoint: unknown key refused", (await post("pk_nope")).status === 404);

  // ------------------------------------------------------------ 2. browser: hydration errors, flash, final text
  browser = await chromium.launch();
  const NEXT_FRONT = await front("es.next.test", NEXT);
  const VITE_FRONT = await front("es.vite.test", VITE);
  const cases: [string, string, string[]][] = [
    ["Next App Router, client component", `${NEXT_FRONT}/app-mixed`, ["Simple pricing for teams", "Cancel anytime"]],
    ["Next App Router, server text only", `${NEXT_FRONT}/app-clean`, ["Simple pricing for teams", "Start for free today.", "Add to cart"]],
    ["Next Pages Router, hard-coded JSX", `${NEXT_FRONT}/pages-mixed`, ["Legacy pricing page", "Cancel anytime"]],
    ["Vite + React 19 SSR (hydrateRoot)", `${VITE_FRONT}/`, ["Simple pricing for teams", "Cancel anytime", "Add to cart"]],
  ];
  for (const b of [true, false]) {
    baseline = b;
    for (const [name, url, sources] of cases) {
      const r = await probe(browser, url, { sources, targets: sources.map((s) => ES[s]!), settle: 2000, locale: "es-ES" });
      const hyd = r.errors.filter((e) => /hydrat|418|423|425|did not match|didn't match|recoverable/i.test(e));
      const ok = !r.errors.length && !r.flashes.length && !r.missing.length;
      const detail = `errors=${r.errors.length} (hydration ${hyd.length}) flash=${JSON.stringify(r.flashes)} untranslated=${JSON.stringify(r.missing)}`;
      if (b) console.log(`info  baseline (JS from the origin) ${name}: ${detail}`);
      else check(`bundles: ${name}: 0 hydration error, 0 flash, translated`, ok, detail + (r.errors.length ? ` first=${r.errors[0]}` : ""));
    }
  }

  // ------------------------------------------------------------ 3. a translation edited in glot reaches the chunk
  psql(`UPDATE edge_translations SET value = 'Cancela cuando lo desees', updated_at = now() WHERE project_id = '${project}' AND hash = '${hash16("en\u0000Cancel anytime")}'`);
  let c2 = c1;
  for (let i = 0; i < 40; i++) {
    c2 = await viaGlot("es.next.test", chunk, { "if-none-match": etag });
    if (c2.status === 200 && c2.body.includes("Cancela cuando lo desees")) break;
    await sleep(500);
  }
  check("new TM version: the browser's copy (old ETag) is replaced, new value served", c2.status === 200 && c2.body.includes("Cancela cuando lo desees") && c2.headers.get("etag") !== etag, `status ${c2.status} etag ${c2.headers.get("etag")} version ${c2.headers.get("x-glot-version")}`);
  const r = await probe(browser, `${NEXT_FRONT}/app-mixed`, { sources: ["Cancel anytime", "Cancela cuando quieras"], targets: ["Cancela cuando lo desees"], settle: 2000, locale: "es-ES" });
  check("after the edit: page and chunk agree (0 hydration error)", !r.errors.length && !r.missing.length && !r.flashes.length, `errors=${r.errors.length} flash=${JSON.stringify(r.flashes)} missing=${JSON.stringify(r.missing)}`);
} catch (e) {
  console.error(e);
  failed = true;
} finally {
  await cleanup();
}
const bad = failed || results.some((ok) => !ok);
console.log(`\n${results.filter(Boolean).length}/${results.length} checks passed`);
process.exit(bad ? 1 : 0);
