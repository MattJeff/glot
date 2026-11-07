// <glot-locale-switcher> helpers: strategy resolution, current locale, navigation target.
import { test } from "node:test";
import assert from "node:assert/strict";
import { currentLocale, match, resolveStrategy, targetUrl, type Manifest } from "../src/locale-switcher.ts";

const m: Manifest = {
  source: "en",
  updated_at: null,
  locales: [
    { code: "en", name: "English", dir: "ltr", coverage: 1 },
    { code: "fr", name: "Français", dir: "ltr", coverage: 1 },
    { code: "pt-BR", name: "Português", dir: "ltr", coverage: 1 },
  ],
};
const codes = m.locales.map((l) => l.code);
const u = (s: string) => new URL(s, "https://site.test");

test("match: exact, then base language", () => {
  assert.equal(match("FR-ca", codes), "fr");
  assert.equal(match("pt", codes), "pt-BR");
  assert.equal(match("de", codes), undefined);
});

test("auto strategy", () => {
  assert.equal(resolveStrategy("auto", u("/?lang=fr"), codes), "query:lang");
  assert.equal(resolveStrategy("auto", u("/fr/pricing"), codes), "prefix");
  assert.equal(resolveStrategy("auto", u("/pricing"), codes), "event");
  assert.equal(resolveStrategy("cookie:NEXT_LOCALE", u("/?lang=fr"), codes), "cookie:NEXT_LOCALE");
});

test("current locale per strategy, then <html lang>, saved, source", () => {
  assert.equal(currentLocale("query:lang", u("/?lang=fr"), "", "en", null, m), "fr");
  assert.equal(currentLocale("query:lang", u("/"), "", "", "fr", m), "fr");
  assert.equal(currentLocale("prefix", u("/pt-BR/x"), "", "en", null, m), "pt-BR");
  assert.equal(currentLocale("prefix", u("/x"), "", "fr", null, m), "en"); // unprefixed = source
  assert.equal(currentLocale("cookie:L", u("/"), "a=1; L=fr", "en", null, m), "fr");
  assert.equal(currentLocale("event", u("/"), "", "fr-FR", null, m), "fr");
  assert.equal(currentLocale("event", u("/"), "", "", null, m), "en");
  assert.equal(currentLocale("event", u("/"), "", "en", "fr", m), "fr"); // a cancelled change still shows the pick
});

test("navigation targets", () => {
  assert.equal(targetUrl("query:lang", u("/p?x=1&lang=en#h"), "fr", m), "https://site.test/p?x=1&lang=fr#h");
  assert.equal(targetUrl("prefix", u("/fr/p?q=1"), "pt-BR", m), "https://site.test/pt-BR/p?q=1");
  assert.equal(targetUrl("prefix", u("/fr/p"), "en", m), "https://site.test/p");
  assert.equal(targetUrl("prefix", u("/p"), "fr", m), "https://site.test/fr/p");
  assert.equal(targetUrl("prefix:always", u("/fr/p"), "en", m), "https://site.test/en/p");
  assert.equal(targetUrl("prefix", u("/fr"), "en", m), "https://site.test/");
  assert.equal(targetUrl("event", u("/"), "fr", m), null);
});

test("element in a real browser: pills, aria-current, keyboard, navigation, catalog", async () => {
  const { chromium } = await import("playwright");
  const { createServer } = await import("node:http");
  const { readFileSync } = await import("node:fs");
  const js = readFileSync(new URL("../dist/locale-switcher.global.js", import.meta.url), "utf8"); // needs `pnpm build`
  const server = createServer((req, res) => {
    const path = new URL(req.url!, "http://x").pathname;
    if (path === "/s.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(js);
    if (path === "/cdn/demo/locales.json") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ ...m, locales: [...m.locales, { code: "ar", name: "العربية", dir: "rtl", coverage: 1 }] }));
    if (path === "/cdn/demo/fr.json") return res.writeHead(200, { "content-type": "application/json" }).end('{"home":{"title":"Bienvenue"}}');
    res.writeHead(200, { "content-type": "text/html" }).end(`<html lang="en"><script src="/s.js" data-project="demo" defer></script><glot-locale-switcher strategy="query:lang"></glot-locale-switcher>`);
  });
  await new Promise<void>((r) => server.listen(0, r));
  const base = `http://localhost:${(server.address() as { port: number }).port}`;
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.goto(`${base}/p?lang=fr`);
    const pills = page.locator("glot-locale-switcher button");
    await pills.first().waitFor();
    assert.deepEqual(await pills.allTextContents(), ["EN", "FR", "PT", "AR"]);
    assert.equal(await pills.nth(1).getAttribute("aria-current"), "true");
    assert.equal(await pills.nth(3).getAttribute("dir"), "rtl");
    assert.equal(await pills.nth(1).getAttribute("aria-label"), "Français");
    await pills.nth(1).focus();
    await page.keyboard.press("ArrowRight");
    assert.equal(await page.evaluate(() => document.querySelector("glot-locale-switcher")!.shadowRoot!.activeElement?.textContent), "PT");
    await Promise.all([page.waitForURL(/lang=en/), pills.first().click()]);
    assert.equal(await page.evaluate(() => localStorage.getItem("glot-locale")), "en");
    const title = await page.evaluate(async () => {
      const got = new Promise((r) => document.addEventListener("glot:catalog", (e) => r((e as CustomEvent).detail.locale)));
      const msgs = await (window as unknown as { glot: { loadCatalog(l: string): Promise<{ home: { title: string } }> } }).glot.loadCatalog("fr");
      return [msgs.home.title, await got];
    });
    assert.deepEqual(title, ["Bienvenue", "fr"]);
  } finally {
    await browser.close();
    server.close();
  }
});
