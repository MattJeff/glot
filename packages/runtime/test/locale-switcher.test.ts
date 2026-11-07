// <glot-locale-switcher> helpers: strategy resolution, current locale, navigation target.
import { test } from "node:test";
import assert from "node:assert/strict";
import { currentLocale, flagFor, match, resolveStrategy, targetUrl, type Manifest } from "../src/locale-switcher.ts";

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

test("flags: language default, region wins, no country → none, overrides", () => {
  assert.equal(flagFor("en"), "🇬🇧");
  assert.equal(flagFor("fr"), "🇫🇷");
  assert.equal(flagFor("ja"), "🇯🇵");
  assert.equal(flagFor("pt-BR"), "🇧🇷");
  assert.equal(flagFor("en-US"), "🇺🇸");
  assert.equal(flagFor("zh-Hant-TW"), "🇹🇼");
  assert.equal(flagFor("es_MX"), "🇲🇽");
  assert.equal(flagFor("es-419"), ""); // a continent, not a country
  for (const c of ["ar", "eo", "la", "ca", "sw"]) assert.equal(flagFor(c), "", c);
  assert.equal(flagFor("en", { en: "🇺🇸" }), "🇺🇸");
  assert.equal(flagFor("en", { en: "us" }), "🇺🇸"); // a region code works too
  assert.equal(flagFor("en-GB", { en: "us" }), "🇬🇧"); // an explicit region beats the language override
  assert.equal(flagFor("ar", { ar: "sa" }), "🇸🇦");
  assert.equal(flagFor("fr", { fr: "" }), ""); // opt out
});

test("element in a real browser: pills, aria-current, keyboard, navigation, catalog", async () => {
  const { chromium } = await import("playwright");
  const { createServer } = await import("node:http");
  const { readFileSync } = await import("node:fs");
  const js = readFileSync(new URL("../dist/locale-switcher.global.js", import.meta.url), "utf8"); // needs `pnpm build`
  let cdnDown = false;
  const server = createServer((req, res) => {
    const path = new URL(req.url!, "http://x").pathname;
    if (cdnDown && path.startsWith("/cdn/")) return res.writeHead(503).end();
    if (path === "/s.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(js);
    if (path === "/cdn/demo/locales.json") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ ...m, locales: [...m.locales, { code: "ar", name: "العربية", dir: "rtl", coverage: 1 }] }));
    if (path === "/cdn/demo/fr.json") return res.writeHead(200, { "content-type": "application/json" }).end('{"home":{"title":"Bienvenue"}}');
    res.writeHead(200, { "content-type": "text/html" }).end(`<html lang="en"><script src="/s.js" data-project="demo" defer></script><glot-locale-switcher strategy="query:lang" variant="pills"></glot-locale-switcher>`);
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
    // glot's CDN down: the last good manifest and catalog (localStorage) keep the switcher and catalogs working.
    cdnDown = true;
    await page.reload();
    await pills.first().waitFor();
    assert.deepEqual(await pills.allTextContents(), ["EN", "FR", "PT", "AR"]);
    const offline = await page.evaluate(() => (window as unknown as { glot: { loadCatalog(l: string): Promise<{ home: { title: string } }> } }).glot.loadCatalog("fr").then((m) => m.home.title));
    assert.equal(offline, "Bienvenue");
  } finally {
    await browser.close();
    server.close();
  }
});

test("menu variant in a real browser: open/close, keyboard, search, selection, flags fallback, RTL, flip", async () => {
  const { chromium } = await import("playwright");
  const { createServer } = await import("node:http");
  const { readFileSync } = await import("node:fs");
  const js = readFileSync(new URL("../dist/locale-switcher.global.js", import.meta.url), "utf8");
  const many: Manifest = {
    source: "en",
    updated_at: null,
    locales: [
      ["en", "English"], ["fr", "Français"], ["de", "Deutsch"], ["es", "Español"], ["it", "Italiano"], ["pt-BR", "Português"],
      ["ja", "日本語"], ["eo", "Esperanto"], ["nl", "Nederlands"], ["ar", "العربية"],
    ].map(([code, name]) => ({ code, name, dir: code === "ar" ? "rtl" : "ltr", coverage: 1 }) as const),
  };
  const server = createServer((req, res) => {
    const u = new URL(req.url!, "http://x");
    if (u.pathname === "/s.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(js);
    if (u.pathname === "/cdn/demo/locales.json") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(many));
    const dir = u.searchParams.get("dir") ?? "ltr";
    const pad = u.searchParams.has("bottom") ? "margin-top:calc(100vh - 40px)" : "margin:0 200px";
    res.writeHead(200, { "content-type": "text/html" }).end(`<html lang="en" dir="${dir}"><body style="margin:0"><script src="/s.js" data-project="demo" defer></script><div style="${pad}"><glot-locale-switcher strategy="query:lang" flag-de="at"></glot-locale-switcher></div><p id="out">outside</p>`);
  });
  await new Promise<void>((r) => server.listen(0, r));
  const base = `http://localhost:${(server.address() as { port: number }).port}`;
  const browser = await chromium.launch();
  // Pretend flag emoji render (a pair is one glyph) or not (Windows: two letters), independent of the CI fonts.
  const emoji = (ok: boolean) => `CanvasRenderingContext2D.prototype.measureText = function (s) { return { width: ${ok} ? 10 : [...s].length * 10 }; };`;
  const active = (page: import("playwright").Page) => page.evaluate(() => {
    const a = document.querySelector("glot-locale-switcher")!.shadowRoot!.activeElement as HTMLElement | null;
    return a?.getAttribute("part")?.split(" ")[0] === "option" ? a.lang : a?.getAttribute("part") ?? null;
  });
  try {
    const page = await browser.newPage();
    await page.addInitScript(emoji(true));
    await page.goto(`${base}/p?lang=es`);
    const trigger = page.locator("glot-locale-switcher [part~=trigger]");
    const menu = page.locator("glot-locale-switcher [part~=menu]");
    const options = page.locator("glot-locale-switcher [role=option]");
    await trigger.waitFor();
    assert.equal((await trigger.textContent())?.trim(), "🇪🇸ES");
    assert.equal(await trigger.getAttribute("aria-haspopup"), "listbox");
    assert.equal(await trigger.getAttribute("aria-expanded"), "false");
    assert.equal(await trigger.getAttribute("aria-label"), "Language: Español (ES)");
    assert.equal(await menu.isVisible(), false);

    // Click opens; > 8 languages: the search field has focus; the current one is selected.
    await trigger.click();
    assert.equal(await trigger.getAttribute("aria-expanded"), "true");
    assert.equal(await options.count(), 10);
    assert.equal(await options.nth(3).getAttribute("aria-selected"), "true");
    assert.equal(await active(page), "search");
    // Flags: language default, flag-de override, region, no country → code badge.
    assert.equal(await options.nth(0).locator("[part~=flag]").textContent(), "🇬🇧");
    assert.equal(await options.nth(2).locator("[part~=flag]").textContent(), "🇦🇹");
    assert.equal(await options.nth(5).locator("[part~=flag]").textContent(), "🇧🇷");
    assert.equal(await options.nth(7).locator("[part~=flag]").textContent(), "EO");
    assert.equal(await options.nth(9).getAttribute("dir"), "rtl");

    // Escape closes and gives focus back to the trigger.
    await page.keyboard.press("Escape");
    assert.equal(await menu.isVisible(), false);
    assert.equal(await active(page), "trigger");

    // Keyboard: ArrowDown opens, arrows / Home / End move, type-ahead, Tab closes.
    await page.keyboard.press("ArrowDown");
    assert.equal(await menu.isVisible(), true);
    await page.keyboard.press("ArrowDown");
    assert.equal(await active(page), "en");
    await page.keyboard.press("ArrowDown");
    assert.equal(await active(page), "fr");
    await page.keyboard.press("End");
    assert.equal(await active(page), "ar");
    await page.keyboard.press("Home");
    assert.equal(await active(page), "en");
    await page.keyboard.press("ArrowUp");
    assert.equal(await active(page), "search");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.type("n");
    assert.equal(await active(page), "nl");
    await page.keyboard.type("j");
    assert.equal(await active(page), "nl"); // "nj" matches nothing: stays
    await page.waitForTimeout(600);
    await page.keyboard.type("i");
    assert.equal(await active(page), "it");
    await page.keyboard.press("Tab");
    assert.equal(await menu.isVisible(), false);

    // Click outside closes.
    await trigger.click();
    await page.locator("#out").click();
    assert.equal(await menu.isVisible(), false);

    // Search (endonym, code, or the visitor's language: "germ" finds Deutsch), then Enter picks → strategy navigates.
    await trigger.click();
    await page.keyboard.type("germ");
    assert.deepEqual(await options.evaluateAll((os) => os.filter((o) => !(o as HTMLElement).hidden).map((o) => (o as HTMLElement).lang)), ["de"]);
    await Promise.all([page.waitForURL(/lang=de/), page.keyboard.press("Enter")]);
    assert.equal(await page.evaluate(() => localStorage.getItem("glot-locale")), "de");
    // Mouse pick.
    await trigger.click();
    await Promise.all([page.waitForURL(/lang=ja/), options.nth(6).click()]);
    assert.equal((await trigger.textContent())?.trim(), "🇯🇵JA");

    // RTL page: the menu opens from the trigger's right edge (reduced motion: no scale-in to wait for).
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`${base}/p?lang=ar&dir=rtl`);
    await trigger.click();
    const [tb, mb] = [await trigger.boundingBox(), await menu.boundingBox()];
    assert.ok(Math.abs(tb!.x + tb!.width - (mb!.x + mb!.width)) < 1, "menu right-aligned in RTL");
    assert.equal(await menu.evaluate((e) => getComputedStyle(e).animationName), "none");
    assert.equal((await trigger.textContent())?.trim(), "AR"); // no country for Arabic: no flag in the trigger

    // Near the viewport bottom: flips up.
    await page.goto(`${base}/p?lang=en&bottom`);
    await trigger.click();
    assert.equal(await menu.evaluate((e) => e.classList.contains("up")), true);
    assert.ok((await menu.boundingBox())!.y < (await trigger.boundingBox())!.y);

    // JS `flags` property overrides.
    await page.evaluate(() => ((document.querySelector("glot-locale-switcher") as unknown as { flags: object }).flags = { en: "🇺🇸" }));
    assert.equal(await trigger.locator("[part~=flag]").textContent(), "🇺🇸");

    // No flag emoji (Windows): code badges, no flag in the trigger.
    const win = await browser.newPage();
    await win.addInitScript(emoji(false));
    await win.goto(`${base}/p?lang=fr`);
    const wt = win.locator("glot-locale-switcher [part~=trigger]");
    await wt.waitFor();
    assert.equal((await wt.textContent())?.trim(), "FR");
    await wt.click();
    assert.equal(await win.locator("glot-locale-switcher [role=option]").nth(1).locator("[part~=flag]").textContent(), "FR");
  } finally {
    await browser.close();
    server.close();
  }
});
