// E2E: static page + `glot dev` on a temp project (mock provider) → Alt+click → popover → edit → file written.
// Needs the glot binary: GLOT_BIN, else target/agent-dev/debug/glot or target/debug/glot.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { createServer, type Server } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { chromium, type Browser } from "playwright";

const repo = resolve(import.meta.dirname, "../../..");
const bin = [process.env.GLOT_BIN, join(repo, "target/debug/glot")].find((p) => p && existsSync(p))!;
const dir = mkdtempSync(join(tmpdir(), "glot-overlay-"));
let glot: ChildProcess, web: Server, browser: Browser, devUrl = "", pageUrl = "";

before(async () => {
  assert.ok(bin, "build glot first: cargo build -p glot-cli");
  mkdirSync(join(dir, "messages"));
  writeFileSync(join(dir, "glot.toml"), 'source_locale = "en"\ntarget_locales = ["fr"]\nfiles = ["messages/{locale}.json"]\n[llm]\nprovider = "mock"\nmodel = "mock"\n');
  writeFileSync(join(dir, "messages/en.json"), JSON.stringify({ home: { title: "Welcome back", greet: "Hello {name}!" } }));
  writeFileSync(join(dir, "messages/fr.json"), JSON.stringify({ home: { title: "Bon retour" } }));
  glot = spawn(bin, ["dev", "--port", "0", "--token", "t0k"], { cwd: dir, env: { ...process.env, NO_COLOR: "1" } });
  devUrl = await new Promise<string>((ok, ko) => {
    glot.stdout!.on("data", (d) => { const m = /on (http:\/\/[\d.:]+)/.exec(String(d)); if (m) ok(m[1]); });
    glot.on("exit", (c) => ko(new Error("glot dev exited " + c)));
  });
  const html = `<!doctype html><html><body style="font-family:sans-serif;padding:40px">
    <h1 id="title">Welcome back</h1><p id="greet">Hello Ana!</p>
    <button id="cta" style="width:60px;white-space:nowrap;overflow:hidden">Start free trial</button>
    <script src="${devUrl}/overlay.js" data-token="t0k"></script></body></html>`;
  web = createServer((_, res) => { res.setHeader("content-type", "text/html"); res.end(html); });
  await new Promise<void>((ok) => web.listen(0, "127.0.0.1", ok));
  pageUrl = `http://127.0.0.1:${(web.address() as any).port}/`;
  browser = await chromium.launch();
});

after(async () => { await browser?.close(); web?.close(); glot?.kill(); });

test("alt+click → popover → edit → catalog written", async () => {
  const page = await browser.newPage();
  await page.goto(pageUrl);
  await page.click("#title", { modifiers: ["Alt"] });
  const pop = page.locator(".pop.open");
  await pop.locator(".title", { hasText: "home.title" }).waitFor();
  const fr = pop.locator(".loc", { hasText: "fr" }).locator("textarea");
  assert.equal(await fr.inputValue(), "Bon retour");
  await fr.fill("Content de te revoir");
  await fr.press("Enter");
  await pop.locator(".status.ok").waitFor();
  assert.match(readFileSync(join(dir, "messages/fr.json"), "utf8"), /Content de te revoir/);

  // invalid edit (placeholder dropped) is rejected by the validator, nothing written
  await page.click("#greet", { modifiers: ["Alt"] });
  await pop.locator(".title", { hasText: "home.greet" }).waitFor();
  const fr2 = pop.locator(".loc", { hasText: "fr" }).locator("textarea");
  await fr2.fill("Bonjour !");
  await fr2.press("Enter");
  await pop.locator(".status.bad").waitFor();
  assert.doesNotMatch(readFileSync(join(dir, "messages/fr.json"), "utf8"), /Bonjour !/);

  // AI: translate everywhere (mock provider)
  await pop.getByText("Translate all").click();
  await pop.getByText(/Translated into 1 locale/).waitFor();
  assert.match(readFileSync(join(dir, "messages/fr.json"), "utf8"), /⟦fr⟧ Hello \{name\}!/);
});

test("hardcoded text → extract to key, overflow flagged", async () => {
  const page = await browser.newPage();
  await page.goto(pageUrl);
  await page.click("#cta", { modifiers: ["Alt"] });
  const pop = page.locator(".pop.open");
  await pop.getByText("Hardcoded text").waitFor();
  await pop.locator("input").fill("home.cta");
  await pop.getByText("Extract to key").click();
  await pop.getByText(/Added/).waitFor();
  assert.match(readFileSync(join(dir, "messages/en.json"), "utf8"), /"cta": "Start free trial"/);
  // now it is a key, and the clipped button is reported
  await page.click("#cta", { modifiers: ["Alt"] });
  await pop.locator(".title", { hasText: "home.cta" }).waitFor();
  await pop.getByText("⚠ overflow").waitFor();
});
