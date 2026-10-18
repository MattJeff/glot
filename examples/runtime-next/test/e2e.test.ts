// Real Next App Router (production build) + fake Translator: the browser asks for `de`, the app ships `en`.
// Server Component text (getGlotTranslations), Client Component text (useTranslations) and the badge must all
// end up in German, glossary respected, with no console error (hydration included).
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";
import { chromium, type Browser } from "playwright";

const PORT = Number(process.env.PORT ?? 3241), url = `http://localhost:${PORT}/`;
let next: ChildProcess, browser: Browser;

// Word-by-word "German". Glossary terms and placeholders arrive masked as ⟦n⟧ and pass through untouched.
const fake = `(() => {
  const dict = { Welcome: "Willkommen", to: "bei", You: "Sie", have: "haben", file: "Datei", files: "Dateien", in: "in",
    your: "Ihrem", This: "Dieser", text: "Text", comes: "kommt", from: "aus", a: "einer", Server: "Server",
    Component: "Komponente", Clicks: "Klicks", Add: "Eins", one: "hinzufügen", Automatically: "Automatisch",
    translated: "übersetzt", on: "auf", device: "Gerät", Improve: "Verbessern" };
  window.Translator = {
    async availability() { return "available"; },
    async create() { return { async translate(s) { return s.replace(/[A-Za-z]+/g, (w) => dict[w] ?? w); } }; },
  };
})()`;

before(async () => {
  const bin = resolve(import.meta.dirname, "../node_modules/.bin/next");
  next = spawn(bin, ["start", "-p", String(PORT)], { cwd: resolve(import.meta.dirname, ".."), stdio: "inherit" });
  for (let i = 0; ; i++) {
    try { if ((await fetch(url)).ok) break; } catch { /* not up yet */ }
    if (i > 100) throw new Error("next start did not come up");
    await new Promise((r) => setTimeout(r, 200));
  }
  browser = await chromium.launch();
});
after(async () => { await browser?.close(); next?.kill(); });

test("server HTML is the shipped English (SEO / no-JS)", async () => {
  const html = await (await fetch(url, { headers: { "accept-language": "de-DE" } })).text();
  assert.match(html, /Welcome to glot, Ana/);
  assert.match(html, /This text comes from a Server Component/);
});

test("de (not shipped): Server + Client Components translated on device, no hydration error", async () => {
  const ctx = await browser.newContext({ locale: "de-DE" });
  const p = await ctx.newPage();
  const errors: string[] = [];
  p.on("console", (m) => { if (m.type() === "error" || m.type() === "warning") errors.push(m.text()); });
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.addInitScript(fake);
  await p.goto(url);

  await p.waitForFunction(() => document.querySelector("h1")?.textContent === "Willkommen bei glot, Ana");
  assert.equal(await p.textContent("#files"), "Sie haben 3 Dateien in Ihrem Arbeitsbereich");
  assert.equal(await p.textContent("#server"), "Dieser Text kommt aus einer Server Komponente");
  assert.equal(await p.textContent("#count"), "Klicks: 0");
  assert.equal(await p.textContent("button"), "Eins hinzufügen");

  // Client state still works on the translated tree.
  await p.click("button");
  await p.waitForFunction(() => document.querySelector("#count")?.textContent === "Klicks: 1");

  await p.waitForSelector("#glot-runtime-badge");
  const visible = await p.evaluate(() => document.body.innerText);
  assert.match(visible, /Automatisch übersetzt auf Ihrem Gerät/);
  // No English word left anywhere on screen ("glot" is do-not-translate, "Server" is the same in German).
  const english = /\b(Welcome|You|have|files?|your|Workspace|This|text|comes|from|Component|Clicks|Add|one|Improve|Automatically)\b/;
  assert.doesNotMatch(visible, english, visible);

  assert.deepEqual(errors, []);
  await ctx.close();
});

test("en browser: untouched English, no badge", async () => {
  const ctx = await browser.newContext({ locale: "en-US" });
  const p = await ctx.newPage();
  await p.addInitScript(fake);
  await p.goto(url);
  await p.waitForLoadState("networkidle");
  assert.equal(await p.textContent("h1"), "Welcome to glot, Ana");
  assert.equal(await p.$("#glot-runtime-badge"), null);
  await ctx.close();
});
