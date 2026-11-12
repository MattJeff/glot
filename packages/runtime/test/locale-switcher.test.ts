// <glot-locale-switcher> helpers: strategy resolution, current locale, navigation target.
import { test } from "node:test";
import assert from "node:assert/strict";
import { applied, currentLocale, flagFor, match, resolveStrategy, targetUrl, type Manifest } from "../src/locale-switcher.ts";

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
    res.writeHead(200, { "content-type": "text/html" }).end(`<html lang="en" dir="${dir}"><body style="margin:0"><script src="/s.js" data-project="demo" defer></script><div style="${pad}"><glot-locale-switcher strategy="query:lang" flag-de="at"></glot-locale-switcher></div><p id="out">outside</p>${u.searchParams.has("two") ? '<glot-locale-switcher strategy="query:lang"></glot-locale-switcher>' : ""}`);
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

    // The app re-creates the element (CSR mount over a pre-rendered page): an open menu survives, with its search,
    // highlighted option and focus; a closed one stays closed; the right one of two is restored.
    await page.goto(`${base}/p?lang=en&two`);
    await trigger.first().waitFor();
    const recreate = (i: number) => page.evaluate((i) => {
      const old = document.querySelectorAll("glot-locale-switcher")[i];
      old.replaceWith(old.cloneNode());
    }, i);
    const state = (i: number) => page.evaluate((i) => {
      const r = document.querySelectorAll("glot-locale-switcher")[i].shadowRoot!;
      const a = r.activeElement as HTMLElement | null;
      return {
        open: !r.querySelector<HTMLElement>("[part~=menu]")?.hidden,
        q: r.querySelector("input")?.value,
        shown: [...r.querySelectorAll<HTMLElement>("[role=option]")].filter((o) => !o.hidden).length,
        focus: a?.lang || a?.getAttribute("part"),
      };
    }, i);
    const settled = () => page.waitForFunction(() => [...document.querySelectorAll("glot-locale-switcher")].every((e) => e.shadowRoot!.querySelector("[part~=trigger]")));
    await trigger.nth(1).click();
    await page.keyboard.type("i");
    await page.keyboard.press("ArrowDown");
    const before = await state(1);
    assert.equal(before.open, true);
    assert.ok(before.shown < 10);
    await recreate(1);
    await settled();
    assert.deepEqual(await state(1), before);
    assert.equal((await state(0)).open, false);
    // Closed, then re-created: stays closed.
    await page.keyboard.press("Escape");
    await recreate(1);
    await settled();
    assert.equal((await state(1)).open, false);
    // Removed while open, then another menu opened before it comes back: not reopened.
    await trigger.nth(1).click();
    await page.evaluate(() => (window as unknown as { gone: Node }).gone = document.querySelectorAll("glot-locale-switcher")[1].cloneNode());
    await page.evaluate(() => document.querySelectorAll("glot-locale-switcher")[1].remove());
    await trigger.first().click();
    await page.evaluate(() => document.body.append((window as unknown as { gone: Node }).gone));
    await settled();
    assert.equal((await state(0)).open, true);
    assert.equal((await state(1)).open, false);
    // Past the 2 s window: not reopened.
    await page.keyboard.press("Escape");
    await trigger.nth(1).click();
    await page.evaluate(() => document.querySelectorAll("glot-locale-switcher")[1].remove());
    await page.waitForTimeout(2100);
    await page.evaluate(() => document.body.append((window as unknown as { gone: Node }).gone.cloneNode()));
    await settled();
    assert.equal((await state(1)).open, false);

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

// Site codes (Settings → Languages in glot): the site reads `zh`, glot calls the locale `zh-CN`.
test("site_code: written in the URL, both codes recognized as the current language", () => {
  const s: Manifest = { source: "fr", updated_at: null, locales: [
    { code: "fr", name: "Français", dir: "ltr", coverage: 1 },
    { code: "zh-CN", name: "中文", dir: "ltr", coverage: 1, site_code: "zh" },
  ] };
  assert.equal(targetUrl("query:lang", u("/?lang=fr"), "zh-CN", s), "https://site.test/?lang=zh");
  assert.equal(targetUrl("prefix", u("/pricing"), "zh-CN", s), "https://site.test/zh/pricing");
  assert.equal(targetUrl("prefix", u("/zh/pricing"), "fr", s), "https://site.test/pricing");
  assert.equal(targetUrl("prefix", u("/zh-CN/pricing"), "fr", s), "https://site.test/pricing");
  for (const q of ["zh", "zh-CN", "ZH"]) assert.equal(currentLocale("query:lang", u(`/?lang=${q}`), "", "fr", null, s), "zh-CN", q);
  assert.equal(currentLocale("prefix", u("/zh/pricing"), "", "", null, s), "zh-CN");
  assert.equal(currentLocale("prefix", u("/zh-CN/pricing"), "", "", null, s), "zh-CN");
  assert.equal(currentLocale("cookie:L", u("/"), "a=1; L=zh", "", null, s), "zh-CN");
  // Without a site code nothing changes.
  assert.equal(targetUrl("query:lang", u("/"), "pt-BR", m), "https://site.test/?lang=pt-BR");
});

test("live check verdict: did the page take the language?", () => {
  const zh = { code: "zh-CN" };
  const before = { l: "fr", p: 1 };
  // The cliponit case: ?lang=zh-CN, the page stays French (same lang, same text).
  assert.equal(applied("fr", zh, before, 1, "fr"), false);
  // <html lang> names the language (glot's code, the site's code, or the same language).
  assert.equal(applied("zh-CN", zh, before, 1, "fr"), true);
  assert.equal(applied("zh", zh, before, 1, "fr"), true);
  assert.equal(applied("ZH-hans", zh, before, 1, "fr"), true);
  assert.equal(applied("cmn", { code: "zh-CN", site_code: "cmn" }, before, 1, "fr"), true);
  // A site that never updates <html lang> but did change its text.
  assert.equal(applied("fr", zh, before, 2, "fr"), true);
  assert.equal(applied("", zh, { l: "", p: 1 }, 2, "fr"), true);
  assert.equal(applied("", zh, { l: "", p: 1 }, 1, "fr"), false);
  // The site fell back to another language than the one asked for (lang moved, not to zh).
  assert.equal(applied("fr", zh, { l: "en", p: 1 }, 2, "fr"), false);
  // Same base language as the source: only the exact code counts as proof.
  assert.equal(applied("pt", { code: "pt-BR" }, { l: "pt", p: 1 }, 1, "pt"), false);
  assert.equal(applied("pt-BR", { code: "pt-BR" }, { l: "pt", p: 1 }, 1, "pt"), true);
});

// The cliponit incident, end to end in a real browser: glot lists zh-CN, the site's own code only accepts
// 2-3 letter codes. The visitor picks 中文, the page stays French → one "not applied" report. Once the project
// maps zh-CN to the site code `zh`, the switcher writes ?lang=zh, the page turns Chinese → "applied".
test("live check in a real browser: a site that ignores zh-CN but honors zh", async () => {
  const { chromium } = await import("playwright");
  const { createServer } = await import("node:http");
  const { readFileSync } = await import("node:fs");
  const js = readFileSync(new URL("../dist/locale-switcher.global.js", import.meta.url), "utf8"); // needs `pnpm build`
  let mapped = false;
  const beacons: Record<string, unknown>[] = [];
  let got = () => {};
  const server = createServer((req, res) => {
    const path = new URL(req.url!, "http://x").pathname;
    if (path === "/s.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(js);
    if (path === "/cdn/clip/locales.json")
      return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ source: "fr", updated_at: null, locales: [
        { code: "fr", name: "Français", dir: "ltr", coverage: 1 },
        { code: "zh-CN", name: "中文", dir: "ltr", coverage: 1, ...(mapped ? { site_code: "zh" } : {}) },
      ] }));
    if (path === "/cdn/clip/live-check" && req.method === "POST") {
      let body = "";
      req.on("data", (c) => (body += c));
      req.on("end", () => (beacons.push(JSON.parse(body)), res.writeHead(204).end(), got()));
      return;
    }
    // The site: client-rendered strings, and a language guard like cliponit's.
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(`<html lang="fr"><script src="/s.js" data-project="clip" defer></script>
<h1 id="h">Crée tes vidéos</h1><glot-locale-switcher strategy="query:lang" variant="pills"></glot-locale-switcher>
<script>const l = new URLSearchParams(location.search).get("lang") || "";
if (/^[a-z]{2,3}$/.test(l) && l === "zh") { document.documentElement.lang = l; h.textContent = "创建你的视频"; }</script>`);
  });
  await new Promise<void>((r) => server.listen(0, r));
  const base = `http://localhost:${(server.address() as { port: number }).port}`;
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    const pick = async (url: RegExp) => {
      await page.goto(`${base}/pricing?lang=fr`);
      const pills = page.locator("glot-locale-switcher button");
      await pills.first().waitFor();
      const beacon = new Promise<void>((r) => (got = r));
      await Promise.all([page.waitForURL(url), pills.nth(1).click()]);
      await pills.first().waitFor();
      await beacon;
      return pills;
    };
    await pick(/lang=zh-CN/);
    assert.equal(await page.textContent("h1"), "Crée tes vidéos"); // translated in glot, not on the site
    assert.deepEqual(beacons, [{ locale: "zh-CN", code: "zh-CN", html_lang: "fr", applied: false, origin: base }]);

    mapped = true; // Settings → Languages: "use zh on my site"
    const pills = await pick(/lang=zh$/);
    assert.equal(await page.textContent("h1"), "创建你的视频");
    assert.equal(await pills.nth(1).getAttribute("aria-current"), "true"); // ?lang=zh is zh-CN
    assert.deepEqual(beacons[1], { locale: "zh-CN", code: "zh", html_lang: "zh", applied: true, origin: base });

    // Once per (project, locale, result) per browser per day: the same outcome again sends nothing.
    await page.goto(`${base}/pricing?lang=fr`);
    await page.locator("glot-locale-switcher button").nth(1).click();
    await page.waitForURL(/lang=zh$/);
    await page.waitForTimeout(4000);
    assert.equal(beacons.length, 2);
  } finally {
    await browser.close();
    server.close();
  }
});
