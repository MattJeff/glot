// Client patcher: the lean DOM segmenter must cut live DOM exactly like the server segmenter cuts HTML (same text,
// role, hash: the TM lookups match), plus re-translation, discovery and version polling in a real Chromium.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { buildSync } from "esbuild";
import { hash16, template } from "../src/patcher.ts";
import { segmentHashed, translateHtml } from "../src/segment.ts";

const FIX = new URL("../../../crates/core/tests/fixtures/edge/", import.meta.url).pathname;
const fixtures = readdirSync(FIX).filter((f) => f.endsWith(".html"));
const PW = new URL("../../renderer/node_modules/playwright/index.mjs", import.meta.url).pathname;
const noBrowser = !existsSync(PW) && "playwright (packages/renderer) not installed";

test("sync SHA-256 = the fixtures' hashes (glot_core::edge::tm::text_hash)", () => {
  for (const f of fixtures) {
    const segs = JSON.parse(readFileSync(FIX + f.replace(/\.html$/, ".segments.json"), "utf8")) as { hash: string; text: string }[];
    for (const s of segs) assert.equal(hash16(`en\u0000${s.text}`), s.hash, `${f}: ${s.text}`);
  }
  assert.equal(hash16(""), "e3b0c44298fc1c149afbf4c8996fb924");
  assert.equal(hash16("é".repeat(100)).length, 32);
});

test("page template = tm.rs template()", () => {
  assert.equal(template("/es/pricing", "es"), "/pricing");
  assert.equal(template("/", "es"), "/");
  assert.equal(template("/blog/42/", "es"), "/blog/:id");
  assert.equal(template("/blog/my-first-post", "es"), "/blog/*");
  assert.equal(template("/explainer-video", "es"), "/explainer-video");
  assert.equal(template("/u/3f2a9c1e-77aa-4b1e-9d2b-0c1d2e3f4a5b/edit", "es"), "/u/:id/edit");
});

const bundle = () =>
  buildSync({
    stdin: { contents: 'import { domSegments, startPatcher } from "./src/patcher.ts"; globalThis.glot = { domSegments, startPatcher };', resolveDir: new URL("..", import.meta.url).pathname, loader: "ts" },
    bundle: true, write: false, format: "iife", target: "es2022",
  }).outputFiles[0].text;

async function browser() {
  const { chromium } = await import(PW); // playwright lives in packages/renderer: untyped here
  return chromium.launch() as Promise<any>;
}

// The one place a browser's HTML parser and the server's tokenizer disagree: malformed entities of the
// edge-cases fixture (`&#0;`, `&#x110000;`, `&AMP;`, `&amp` without `;` — the browser decodes them all).
const parserDiff = (text: string) => text.startsWith("Entities: ");

test("DOM segmenter = server segmenter on every shared fixture (text, role, hash, discoverable)", { skip: noBrowser }, async () => {
  const b = await browser();
  try {
    const page = await b.newPage();
    const js = bundle();
    for (const f of fixtures) {
      const html = readFileSync(FIX + f, "utf8");
      await page.setContent(html.replace(/<script\b[\s\S]*?<\/script>/gi, (s) => s.replace(/<script/i, '<script type="text/x-inert"')));
      await page.addScriptTag({ content: js });
      const dom: { text: string; role: string; hash: string; discoverable: boolean }[] = await page.evaluate(() =>
        (globalThis as any).glot.domSegments(document.documentElement, "en").map((s: any) => ({ text: s.text, role: s.role, hash: s.hash, discoverable: s.discoverable })));
      // <noscript> is raw text in a browser with JS (never shown, never patched): not compared
      const ns = [...html.matchAll(/<noscript[\s\S]*?<\/noscript>/gi)].map((m) => [m.index!, m.index! + m[0].length]);
      const want = segmentHashed(html, { sourceLocale: "en" })
        .filter((s) => !ns.some(([a, b]) => s.span[0] >= a && s.span[1] <= b) && !parserDiff(s.text))
        .map((s) => JSON.stringify({ text: s.text, role: s.role, hash: s.hash, discoverable: s.discoverable }));
      const got = dom.filter((s) => !parserDiff(s.text)).map((s) => JSON.stringify(s));
      const lacks = want.filter((x) => !got.includes(x)), extra = got.filter((x) => !want.includes(x));
      assert.deepEqual({ lacks, extra }, { lacks: [], extra: [] }, f);
    }
  } finally {
    await b.close();
  }
});

test("patcher writes the same translation into the live DOM as the server writes into the HTML (every fixture)", { skip: noBrowser }, async () => {
  const b = await browser();
  // whitespace and comments aside: the patcher keeps framework comment markers (Leptos `<!---->`) the server drops
  const flat = (h: string) => h.replace(/<!--[\s\S]*?-->/g, "").replace(/\s+/g, " ").replace(/> /g, ">").replace(/ </g, "<");
  try {
    const page = await b.newPage();
    const js = bundle();
    const inert = (h: string) => h.replace(/<script\b[\s\S]*?<\/script>/gi, (x) => x.replace(/<script/i, '<script type="text/x-inert"')).replace(/<noscript[\s\S]*?<\/noscript>/gi, "");
    for (const f of fixtures) {
      // parserDiff, and an unclosed <b> the browser re-parents (adoption agency) differently around the server's edit
      const html = inert(readFileSync(FIX + f, "utf8")).replace(/<p>Entities: .*<\/p>/, "").replace("<p>Unclosed <b>bold to the end</p>", "");
      const t: Record<string, string> = {};
      for (const sg of segmentHashed(html, { sourceLocale: "en" })) t[sg.hash] = `ES[${sg.text}]`;
      const server = translateHtml(html, { sourceLocale: "en" }, (sg) => t[sg.hash]);
      await page.setContent(server);
      const want = flat(await page.evaluate(() => document.body.outerHTML));
      await page.setContent(html);
      await page.addScriptTag({ content: js });
      const got = flat(await page.evaluate((tm: string) => {
        document.querySelectorAll("script:not([type])").forEach((x) => x.remove());
        (globalThis as any).glot.startPatcher({ source: "en", inline: tm });
        return document.body.outerHTML;
      }, JSON.stringify({ v: 1, locale: "es", t })));
      assert.equal(got, want, f);
    }
  } finally {
    await b.close();
  }
});

test("patcher: re-translates client re-renders in place, discovers client-only text once, applies the new TM version", { skip: noBrowser }, async () => {
  const b = await browser();
  const h = (t: string) => hash16(`en\u0000${t}`);
  let v = 3;
  const tm = (): Record<string, unknown> => ({ v, locale: "es", t: { [h("Save")]: { "button|/cliping": "Guardar", "*": "Ahorrar" }, [h("Pricing for <0>teams</0>")]: "Precios para <0>equipos</0>", [h("Search")]: "Buscar", ...(v > 3 ? { [h("Play")]: "Reproducir" } : {}) } });
  const posts: any[] = [];
  try {
    const ctx = await b.newContext();
    const page = await ctx.newPage();
    await page.route("https://site.test/**", async (r: any) => {
      const u = new URL(r.request().url());
      if (u.pathname === "/__glot/tm.json") {
        if (Number(u.searchParams.get("since") ?? -1) >= v) return r.fulfill({ status: 204 });
        return r.fulfill({ contentType: "application/json", body: JSON.stringify(tm()) });
      }
      if (u.pathname === "/__glot/missing") return posts.push(JSON.parse(r.request().postData()!)), r.fulfill({ status: 204 });
      // Edge-translated server HTML: already Spanish; the app below re-renders parts of it in English.
      return r.fulfill({ contentType: "text/html", body: `<!doctype html><html lang="es"><head><meta name="glot-tm" content="es@3"></head><body>
        <h2>Precios para <b>equipos</b></h2><div id="app"><button>Guardar</button></div><input placeholder="Buscar">
        <p data-glot-user>Hola Ana</p><div id="late"></div></body></html>` });
    });
    await page.goto("https://site.test/es/cliping");
    await page.addScriptTag({ content: bundle() });
    await page.evaluate(() => {
      (globalThis as any).p = (globalThis as any).glot.startPatcher({ source: "en", tmUrl: "/__glot/tm.json?locale=es", missing: "/__glot/missing", pageVersion: 3, poll: [300, 600] });
    });
    await page.waitForFunction(() => (globalThis as any).p.tm);
    const html = await page.evaluate(async () => {
      const app = document.getElementById("app")!;
      // React-style client render of the source text: a new subtree, then a text update in place.
      app.innerHTML = '<button data-play-label="Play">Save</button>';
      document.querySelector("h2")!.innerHTML = "Pricing for <b>teams</b>";
      document.querySelector("input")!.setAttribute("placeholder", "Search");
      await Promise.resolve();
      // The /cliping case: the app copies a data-* label into the button text (client-only string).
      const btn = app.querySelector("button")!;
      btn.textContent = btn.dataset.playLabel!;
      const late = document.getElementById("late")!;
      late.innerHTML = "<p>Write to ana@private.io</p><p>Brand new client text</p>";
      document.querySelector("[data-glot-user]")!.textContent = "Hello Ana";
      await new Promise((r) => setTimeout(r, 0));
      return document.body.innerHTML;
    });
    assert.match(html, /<h2>Precios para <b>equipos<\/b><\/h2>/);
    assert.match(html, /placeholder="Buscar"/);
    assert.match(html, /data-play-label="Play">Play</); // not in the TM yet
    assert.match(html, /Hello Ana/); // user content: never touched
    await page.waitForTimeout(2000); // debounce
    assert.equal(posts.length, 1, JSON.stringify(posts));
    const texts = posts[0].segments.map((s: any) => s.text).sort();
    assert.deepEqual(texts, ["Brand new client text", "Play"]); // no PII, no user zone, nothing translated
    assert.equal(posts[0].locale, "es");
    assert.equal(posts[0].segments[0].url, "/es/cliping");
    assert.equal(posts[0].segments.find((s: any) => s.text === "Play").hash, h("Play"));
    // glot translates it: the poll picks up v4 and applies it in place.
    v = 4;
    await page.waitForFunction(() => document.querySelector("#app button")!.textContent === "Reproducir", null, { timeout: 3000 });
    // Same text again: never re-reported in this page view.
    await page.evaluate(() => { document.getElementById("late")!.innerHTML = "<p>Brand new client text</p>"; });
    await page.waitForTimeout(1800);
    assert.equal(posts.length, 1);

    // Signed in (session cookie): nothing discovered.
    await ctx.addCookies([{ name: "session_id", value: "x", url: "https://site.test" }]);
    v = 3;
    posts.length = 0;
    await page.goto("https://site.test/es/cliping");
    await page.addScriptTag({ content: bundle() });
    await page.evaluate(() => { (globalThis as any).p = (globalThis as any).glot.startPatcher({ source: "en", tmUrl: "/__glot/tm.json?locale=es", missing: "/__glot/missing", pageVersion: 3 }); });
    await page.waitForFunction(() => (globalThis as any).p.tm);
    await page.evaluate(() => { document.getElementById("late")!.innerHTML = "<p>Another client text</p>"; });
    await page.waitForTimeout(1800);
    assert.equal(posts.length, 0);
  } finally {
    await b.close();
  }
});
