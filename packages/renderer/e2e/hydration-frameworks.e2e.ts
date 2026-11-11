// SSR frameworks behind the real glot Edge JS runtime (packages/edge: createEdge + translate.ts, subdomain-style
// proxy on the same paths): does the Spanish page hydrate without console errors, without a painted frame of
// English, with `<html lang="es">`, and does the app keep working (identifier values untouched, click handler)?
//   node e2e/hydration-frameworks.e2e.ts                 (all; FRAMEWORKS=sveltekit,nuxt to filter; STRICT=1 → exit 1 on FAIL)
//   NO_PAYLOAD=1 …   control run: the HTML is translated but not the serialized payloads (what a plain proxy does)
// Fixtures (built once: `npm i && npm run build` in each): e2e/fixtures/{sveltekit,nuxt,react-router,solidstart,astro}-app.
// Each page has: server-rendered text from server data (title, plan CTA → embedded payload), text hard-coded in a
// component (in the HTML *and* the client bundle), text rendered only after mount, and an identifier value
// (`plan: "Pro"`, used as a CSS class) whose source string is in the TM: it must stay "Pro".
// glot itself is simulated (status, TM, patcher bundle): nothing leaves the box.
import { createServer, type Server } from "node:http";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";
import { chromium } from "playwright";
import { createEdge } from "../../edge/src/edge.ts";
import { translate } from "../../edge/src/translate.ts";
import { hash16 } from "../../edge/src/segment.ts";
import { probe } from "./lib.ts";

const FIX = new URL("./fixtures/", import.meta.url).pathname;
const PATCHER = new URL("../../../web/public/widget/edge-patcher.global.js", import.meta.url).pathname;
const ES: Record<string, string> = {
  "Simple pricing for teams": "Precios simples para equipos",
  "Start for free today.": "Empieza gratis hoy.",
  "Add to cart": "Añadir al carrito",
  "Cancel anytime": "Cancela cuando quieras",
  "Ready when you are": "Cuando quieras",
  "Secure payment": "Pago seguro",
  "No credit card required": "Sin tarjeta de crédito",
  "Questions? Write to us": "¿Preguntas? Escríbenos",
  Pro: "Profesional", // the identifier guard: `plan: "Pro"` is a class name, never translated
};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const listen = (s: Server, port = 0) => new Promise<number>((r) => s.listen(port, "127.0.0.1", () => r((s.address() as { port: number }).port)));

// ---------------------------------------------------------------- simulated glot server
const tm = { v: 1, locale: "es", t: Object.fromEntries(Object.entries(ES).map(([s, v]) => [hash16(`en\u0000${s}`), v])) };
const glot = async (input: RequestInfo | URL): Promise<Response> => {
  const p = new URL(String(input instanceof Request ? input.url : input)).pathname;
  const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { "content-type": "application/json" } });
  if (p.endsWith("/status")) return json({ enabled: true, off: [], versions: { es: 1 } });
  if (p.endsWith("/es/tm")) return json(tm);
  if (p === "/edge/patcher.js") return new Response(readFileSync(PATCHER, "utf8"), { headers: { "content-type": "text/javascript" } });
  if (p.endsWith("/locales.json")) return json({ locales: [{ code: "en", ready: true }, { code: "es", ready: true }] });
  return new Response(null, { status: 204 });
};

// ---------------------------------------------------------------- frameworks
/** `paths`: page → what it is made of (the label of the report row). */
interface Fw { name: string; dir: string; cmd?: string[]; static?: string; checks: string[]; sources: string[]; paths: Record<string, string> }
const ALL = ["Simple pricing for teams", "Start for free today.", "Add to cart", "Cancel anytime", "Ready when you are"];
const MIXED = "data + hard-coded + after-mount";
const VUE = [...ALL, "Secure payment", "No credit card required", "Questions? Write to us"];
const FWS: Fw[] = [
  { name: "sveltekit", dir: "sveltekit-app", cmd: ["node", "build"], checks: [".Pro"], sources: ALL, paths: { "/": MIXED } },
  { name: "nuxt", dir: "nuxt-app", cmd: ["node", ".output/server/index.mjs"], checks: [".Pro"], sources: ALL, paths: { "/": MIXED } },
  { name: "react-router", dir: "react-router-app", cmd: ["node_modules/.bin/react-router-serve", "./build/server/index.js"], checks: [".Pro"], sources: ALL, paths: { "/": MIXED, "/clean": "data + after-mount" } },
  { name: "solidstart", dir: "solidstart-app", cmd: ["node", ".output/server/index.mjs"], checks: [".Pro"], sources: ALL, paths: { "/": MIXED } },
  { name: "astro", dir: "astro-app", static: "dist", checks: [".Pro", ".info"], sources: VUE, paths: { "/": `React+Vue islands, ${MIXED}`, "/clean/": "React island data-only + Vue island" } },
];
const NO_PAYLOAD = !!process.env.NO_PAYLOAD;
/** Control = a plain translating proxy: script bodies and island props masked while translating (only the HTML
 * text changes), no Vue `data-allow-mismatch`. The patcher is still injected. */
const htmlOnly: typeof translate = (html, t, c) => {
  const kept: string[] = [];
  const mask = (v: string) => `\u0000glot${kept.push(v) - 1}\u0000`;
  const m = html.replace(/(<script\b[^>]*>)([\s\S]*?)(<\/script>)/g, (_, o: string, b: string, e: string) => o + mask(b) + e).replace(/(\sprops=")([^"]*)/g, (_, o: string, v: string) => o + mask(v));
  const r = translate(m, t, c);
  return { ...r, html: r.html.replace(/\u0000glot(\d+)\u0000/g, (_, i: string) => kept[+i]).replace(' data-allow-mismatch="text"', "") };
};
const only = process.env.FRAMEWORKS?.split(",");

const MIME: Record<string, string> = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml" };
async function startOrigin(fw: Fw): Promise<{ url: string; stop: () => void }> {
  const dir = join(FIX, fw.dir);
  if (fw.static) {
    const root = join(dir, fw.static);
    const s = createServer((req, res) => {
      let p = join(root, decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname));
      if (existsSync(p) && statSync(p).isDirectory()) p = join(p, "index.html");
      if (!existsSync(p)) return res.writeHead(404).end();
      res.writeHead(200, { "content-type": MIME[extname(p)] ?? "application/octet-stream" }).end(readFileSync(p));
    });
    return { url: `http://127.0.0.1:${await listen(s)}`, stop: () => s.close() };
  }
  const probePort = createServer();
  const port = await listen(probePort);
  probePort.close();
  const child: ChildProcess = spawn(fw.cmd![0], fw.cmd!.slice(1), { cwd: dir, env: { ...process.env, PORT: String(port), HOST: "127.0.0.1", NODE_ENV: "production" }, stdio: ["ignore", "ignore", "inherit"] });
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 150; i++) {
    if (await fetch(url + "/").then((r) => r.ok, () => false)) return { url, stop: () => child.kill("SIGTERM") };
    await sleep(100);
  }
  child.kill("SIGTERM");
  throw new Error(`${fw.name}: origin did not start (built? npm run build in ${dir})`);
}

/** The glot Edge in front of `origin`: every path served in Spanish (subdomain mode), `/__glot/*` = its own. */
async function startEdge(origin: string) {
  const edge = createEdge({ server: "https://glot.test", project: "e2e", source: "en", locales: ["es"], translate: NO_PAYLOAD ? htmlOnly : translate, fetch: glot as typeof fetch, timeout: 2000 });
  const s = createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", `http://${req.headers.host}`);
    const headers = new Headers();
    for (const [k, v] of Object.entries(req.headers)) if (typeof v === "string") headers.set(k, v);
    const r = new Request(url, { method: req.method, headers });
    const out = (url.pathname.startsWith("/__glot/") && (await edge.own(r))) ||
      (await edge(r, "es", new URL(url.pathname + url.search, origin), (u) => {
        const h = new Headers(headers);
        h.set("accept-encoding", "identity"); // as express.ts: Node's fetch would decode yet keep content-encoding
        h.delete("host");
        return fetch(u, { method: req.method, headers: h, redirect: "manual" });
      }));
    const h: Record<string, string> = {};
    out.headers.forEach((v, k) => (h[k] = v));
    res.writeHead(out.status, h).end(Buffer.from(await out.arrayBuffer()));
  });
  return { url: `http://127.0.0.1:${await listen(s)}`, stop: () => s.close() };
}

const browser = await chromium.launch();
const rows: string[] = [];
let failed = 0;
for (const fw of FWS.filter((f) => !only || only.includes(f.name))) {
  let origin: Awaited<ReturnType<typeof startOrigin>> | undefined, edge: Awaited<ReturnType<typeof startEdge>> | undefined;
  try {
    origin = await startOrigin(fw);
    edge = await startEdge(origin.url);
    for (const [path, label] of Object.entries(fw.paths)) {
      const first = await fetch(edge.url + path).then((r) => r.text());
      const noJs = fw.sources.filter((s) => !s.startsWith("Ready") && !s.startsWith("Questions") && first.includes(`>${ES[s]}<`)).length;
      let after: { checks: Record<string, boolean>; clicked: boolean } | undefined;
      const r = await probe(browser, edge.url + path, {
        sources: fw.sources,
        targets: fw.sources.map((s) => ES[s]),
        settle: 2500,
        act: async (p) => {
          await p.waitForTimeout(2000);
          const before = await p.evaluate(() => document.querySelector("[data-n]")?.getAttribute("data-n"));
          await p.locator("button").first().click();
          await p.waitForTimeout(200);
          after = await p.evaluate(({ checks, before }) => ({
            checks: Object.fromEntries(checks.map((c) => [c, !!document.querySelector(c)])),
            clicked: before === null || before === undefined || document.querySelector("[data-n]")?.getAttribute("data-n") !== before,
          }), { checks: fw.checks, before });
        },
      });
      const hyd = r.errors.filter((e) => /hydrat|418|423|425|mismatch|did not match|didn't match/i.test(e));
      const ids = Object.entries(after?.checks ?? {}).filter(([, v]) => !v).map(([k]) => k);
      const ok = !r.errors.length && !r.flashes.length && !r.missing.length && r.lang === "es" && !ids.length && !!after?.clicked;
      if (!ok) failed++;
      rows.push(`${ok ? "PASS" : "FAIL"}  ${fw.name.padEnd(13)} ${path.padEnd(8)} ${label.padEnd(44)} no-JS translated=${noJs}/${fw.sources.length - fw.sources.filter((s) => s.startsWith("Ready") || s.startsWith("Questions")).length}  lang=${r.lang}  errors=${r.errors.length} (hydration ${hyd.length})  flash=${JSON.stringify(r.flashes)}  untranslated-at-end=${JSON.stringify(r.missing)}  ids-broken=${JSON.stringify(ids)}  click=${after?.clicked}`);
      if (process.env.VERBOSE) rows.push(...r.errors.map((e) => `      ${e}`), ...r.samples.map((s) => `      flash in: ${s.slice(0, 300)}`));
    }
  } catch (e) {
    failed++;
    rows.push(`FAIL  ${fw.name.padEnd(13)} ${(e as Error).message}`);
  } finally {
    edge?.stop();
    origin?.stop();
  }
}
await browser.close();
console.log(rows.join("\n"));
process.exit(failed && process.env.STRICT ? 1 : 0);
