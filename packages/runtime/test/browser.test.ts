// Chromium headless has no on-device model: a fake `Translator` is injected before the page loads.
// Needs `pnpm build:wasm && pnpm build`.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { chromium, type Browser, type Page } from "playwright";

const dist = resolve(import.meta.dirname, "../dist");
let server: Server, browser: Browser, url = "";

// Fake Translator (runs in the page). Word-by-word "Polish"; text containing BREAK loses its tokens.
const fake = (availability: string) => `(() => {
  const dict = { Hello: "Cześć", You: "Masz", have: "", file: "plik", files: "pliki", in: "w", your: "twoim", Open: "Otwórz", "Save changes": "Zapisz zmiany" };
  window.__calls = 0;
  let av = ${JSON.stringify(availability)};
  // Playwright's evaluate() counts as a user gesture: model "activation" as a real click within the last second.
  let clicked = 0;
  addEventListener("click", () => clicked = Date.now(), true);
  Object.defineProperty(navigator, "userActivation", { get: () => ({ isActive: Date.now() - clicked < 1000 }) });
  window.Translator = {
    async availability() { return av; },
    async create({ monitor }) {
      if (av !== "available" && !navigator.userActivation.isActive) throw new DOMException("activation", "NotAllowedError");
      const t = new EventTarget(); monitor?.(t);
      for (const loaded of [0, 0.5, 1]) { t.dispatchEvent(Object.assign(new Event("downloadprogress"), { loaded })); await new Promise(r => setTimeout(r, 30)); }
      av = "available";
      return { async translate(s) {
        window.__calls++;
        return s.split("\\n").map(line => line.includes("BREAK") ? line.replace(/⟦\\d+⟧/g, "")
          : dict[line] ?? line.replace(/[A-Za-z]+/g, w => dict[w] ?? w).replace(/  +/g, " ")).join("\\n");
      } };
    },
  };
})()`;

const source = {
  home: { greet: "Hello {name}", files: "You have {n, plural, one {# file} other {# files}} in your Workspace", save: "Save changes" },
  broken: "Open BREAK {thing}",
  brand: "Open glot",
};
const glossary = { term: [{ term: "Workspace", translations: { pl: "Obszar roboczy" } }, { term: "glot", do_not_translate: true }] };

before(async () => {
  assert.ok(existsSync(join(dist, "glot.wasm")) && existsSync(join(dist, "index.js")), "pnpm build:wasm && pnpm build");
  server = createServer((req, res) => {
    const p = (req.url ?? "/").split("?")[0];
    if (p === "/") return res.writeHead(200, { "content-type": "text/html" }).end("<!doctype html><body><h1>t</h1>");
    const f = join(dist, p);
    if (!f.startsWith(dist) || !existsSync(f)) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": f.endsWith(".wasm") ? "application/wasm" : "text/javascript" }).end(readFileSync(f));
  }).listen(0);
  await new Promise((r) => server.once("listening", r));
  url = `http://localhost:${(server.address() as { port: number }).port}/`;
  browser = await chromium.launch();
});
after(async () => { await browser?.close(); server?.close(); });

async function page(init?: string): Promise<Page> {
  const ctx = await browser.newContext({ locale: "pl-PL" });
  const p = await ctx.newPage();
  if (init) await p.addInitScript(init);
  await p.goto(url);
  return p;
}
const run = (p: Page) => p.evaluate(async ([source, glossary]) => {
  const rt = await import("/index.js");
  return rt.createFallbackMessages({ source, glossary, locale: "pl" });
}, [source, glossary] as const);

test("no Translator API: clean no-op, source messages", async () => {
  // Recent headless Chromium exposes a Translator without a model: hide it to model Firefox/Safari/mobile.
  const p = await page("delete window.Translator");
  const r = await run(p);
  assert.equal(r.status, "unsupported");
  assert.deepEqual(r.messages, source);
  await p.context().close();
});

test("translates, masks, keeps glossary + plural structure, validates, caches", async () => {
  const p = await page(fake("available"));
  const r = await run(p);
  assert.equal(r.status, "translated");
  const m = r.messages as any;
  assert.equal(m.home.greet, "Cześć {name}");
  assert.equal(m.home.save, "Zapisz zmiany");
  assert.equal(m.home.files, "Masz {n, plural, one {# plik} few {# pliki} many {# pliki} other {# pliki}} w twoim Obszar roboczy");
  assert.equal(m.brand, "Otwórz glot");
  assert.equal(m.broken, source.broken, "lost placeholder → source fallback");
  assert.deepEqual(r.fellBack, ["broken"]);
  const calls = await p.evaluate(() => (window as any).__calls);
  const again = await run(p);
  assert.deepEqual(again.messages, r.messages);
  assert.ok(again.cached > 0);
  assert.equal(await p.evaluate(() => (window as any).__calls), calls + 1, "only the failed segment is retranslated");
  await p.context().close();
});

test("model not downloaded: badge asks for a click, shows progress, then translates", async () => {
  const p = await page(fake("downloadable"));
  await p.evaluate(async ([source, glossary]) => {
    const rt = await import("/index.js");
    const r = await rt.fallbackWithBadge({
      source, glossary, locale: "pl", badge: { improveUrl: "https://example.com/improve" },
      onMessages: (m: any) => { document.querySelector("h1")!.textContent = m.home.save; },
    });
    (window as any).__first = r.status;
  }, [source, glossary] as const);
  assert.equal(await p.evaluate(() => (window as any).__first), "needs-activation");
  await p.click("#glot-runtime-badge span");
  await p.waitForFunction(() => document.querySelector("h1")!.textContent === "Zapisz zmiany");
  const badge = await p.textContent("#glot-runtime-badge");
  assert.match(badge!, /Improve/);
  assert.equal(await p.getAttribute("#glot-runtime-badge a", "href"), "https://example.com/improve");
  await p.context().close();
});
