// Next.js (App Router + Pages Router, React 19) behind a translating proxy: does the page hydrate without errors
// or flash when the Edge translates the HTML (a) alone, (b) + the serialized payloads (hydration.ts), (c) + patcher?
//   NEXT_URL=http://127.0.0.1:4620 node e2e/hydration-next.e2e.ts      (fixture: e2e/fixtures/next-app, `next build && next start`)
// The HTML step here is a stand-in for the Edge segment translator (exact text-node replacement of a fixed
// dictionary), so the test isolates the hydration question from the segmenter.
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { chromium } from "playwright";
import { translatePayloads } from "../../edge/src/hydration.ts";
import { hash16 } from "../../edge/src/patcher.ts";
import { probe } from "./lib.ts";

const NEXT = process.env.NEXT_URL ?? "http://127.0.0.1:4620";
const ES: Record<string, string> = {
  "Simple pricing for teams": "Precios simples para equipos",
  "Start for free today.": "Empieza gratis hoy.",
  "Add to cart": "Añadir al carrito",
  "Cancel anytime": "Cancela cuando quieras",
  "Legacy pricing page": "Página de precios clásica",
  "Subscribe now": "Suscríbete ahora",
};
const PATCHER = process.env.PATCHER ?? new URL("../../../web/public/widget/edge-patcher.global.js", import.meta.url).pathname;
const esc = (s: string) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

type Mode = "html" | "payload" | "payload+patcher";
function translate(html: string, mode: Mode) {
  for (const [s, t] of Object.entries(ES)) html = html.split(`>${esc(s)}<`).join(`>${esc(t)}<`);
  if (mode !== "html") html = translatePayloads(html, (s) => ES[s]).html;
  if (mode === "payload+patcher") {
    // the real patcher bundle + an inline TM in the wire format ({v, locale, t: {hash: value}}), as the Edge injects them
    const t = Object.fromEntries(Object.entries(ES).map(([s, v]) => [hash16(`en\u0000${s}`), v]));
    const tm = JSON.stringify({ v: 1, locale: "es", t }).replace(/</g, "\\u003c");
    html = html.replace(/<head>/, `<head><meta name="glot-tm" content="es@1"><script type="application/json" id="glot-tm">${tm}</script><script data-source="en">${readFileSync(PATCHER, "utf8")}</script>`);
  }
  return html;
}

let mode: Mode = "html";
const proxy = createServer(async (req, res) => {
  const up = await fetch(NEXT + req.url, { headers: { accept: req.headers.accept ?? "*/*" } });
  const type = up.headers.get("content-type") ?? "";
  res.writeHead(up.status, { "content-type": type });
  if (!type.includes("text/html")) return res.end(Buffer.from(await up.arrayBuffer()));
  res.end(translate(await up.text(), mode));
});
await new Promise<void>((r) => proxy.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${(proxy.address() as { port: number }).port}`;

const browser = await chromium.launch();
const rows: string[] = [];
let failed = 0;
const cases: [string, string[], Mode[]][] = [
  ["/app-clean", ["Simple pricing for teams", "Start for free today.", "Add to cart"], ["html", "payload", "payload+patcher"]],
  ["/app-mixed", ["Simple pricing for teams", "Cancel anytime"], ["payload", "payload+patcher"]],
  ["/pages-clean", ["Legacy pricing page", "Subscribe now"], ["html", "payload"]],
  ["/pages-mixed", ["Legacy pricing page", "Cancel anytime"], ["payload", "payload+patcher"]],
];
for (const [path, sources, modes] of cases)
  for (const m of modes) {
    mode = m;
    const r = await probe(browser, base + path, { sources, targets: sources.map((s) => ES[s]) });
    const hydrationErr = r.errors.filter((e) => /hydrat|418|423|425|did not match|didn't match/i.test(e));
    const ok = !r.errors.length && !r.flashes.length && !r.missing.length;
    rows.push(`${ok ? "PASS" : "FAIL"}  ${path.padEnd(13)} ${m.padEnd(16)} errors=${r.errors.length} (hydration ${hydrationErr.length})  flash=${JSON.stringify(r.flashes)}  untranslated-at-end=${JSON.stringify(r.missing)}`);
    if (process.env.VERBOSE) rows.push(...r.errors.map((e) => `      ${e}`));
    // expected failures are documented, not hidden: html-only and hard-coded client text are known limits
    if (!ok && process.env.STRICT) failed++;
  }
await browser.close();
proxy.close();
console.log(rows.join("\n"));
process.exit(failed ? 1 : 0);
