// Universal website tag (widget/glot.js) end to end in a real Chromium, on a static "site builder" page (generic
// div soup, a contact form, a member greeting, SPA navigation through history.pushState) and a fake glot server
// that records every request:
//  1. visitor in the source language: page untouched, the switcher floats in its corner, no TM fetched;
//  2. pick Spanish: translated in place (no reload), <html lang>, <title> and meta description; unknown public
//     text reported (`mode: "website"`, key), never the form's own text, typed values, personal data, query
//     strings or cookies; an SPA route change is translated and its new text reported with its path;
//  3. the choice is remembered (reload → Spanish), `?lang=` wins;
//  4. a session cookie → no discovery; tag off → nothing at all;
//  5. SEO-ready Edge domain: hreflang links on the source page (kept in sync on pushState), the switcher sends the
//     visitor to the subdomain.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type Server } from "node:http";
import { existsSync } from "node:fs";
import { buildSync } from "esbuild";
import { hash16 } from "../src/segment.ts";

const PW = new URL("../../renderer/node_modules/playwright/index.mjs", import.meta.url).pathname;
const noBrowser = !existsSync(PW) && "playwright (packages/renderer) not installed";
const ROOT = new URL("..", import.meta.url).pathname;
const JS = buildSync({ entryPoints: [ROOT + "src/website.global.ts"], bundle: true, write: false, format: "iife", target: "es2022" }).outputFiles[0].text;
const h = (t: string) => hash16(`en\u0000${t}`);
const PII = ["hello@shop.test", "jane.doe@example.com", "Jane Doe", "+33 6 12 34 56 78", "12 rue de la Paix", "utm_source", "s3cr3t"];

const listen = (s: Server) => new Promise<number>((ok) => s.listen(0, "127.0.0.1", () => ok((s.address() as { port: number }).port)));
const body = (req: IncomingMessage) => new Promise<string>((ok) => { let b = ""; req.on("data", (c) => (b += c)).on("end", () => ok(b)); });

const TM = {
  v: 3, locale: "es",
  t: {
    [h("Handmade ceramics")]: "Cerámica artesanal", [h("Shop")]: "Tienda", [h("About")]: "Nosotros",
    [h("Bowls, mugs and plates thrown by hand in Lyon.")]: "Cuencos, tazas y platos torneados a mano en Lyon.",
    [h("Fired at <0>{0} °C</0> for strength.")]: "Cocidos a <0>{0} °C</0> para mayor resistencia.",
    [h("Ceramics — Atelier")]: "Cerámica — Atelier", [h("Pottery made to last.")]: "Cerámica hecha para durar.",
    [h("Our story")]: "Nuestra historia", [h("Send")]: "Enviar",
  },
};

/** The site: one static page, builder-style markup, a form, a member greeting, a client-side route to /about. */
function site(tag: () => string) {
  return createServer((req, res) => {
    if (!req.url!.startsWith("/") || req.url!.startsWith("/favicon")) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(`<!doctype html><html lang="en"><head>
<meta charset="utf-8"><title>Ceramics — Atelier</title><meta name="description" content="Pottery made to last.">
${tag()}</head><body>
<div id="SITE_HEADER" class="wixui-header"><div class="comp-1"><nav><a href="/" data-route>Shop</a> <a href="/about" data-route>About</a></nav></div></div>
<main id="PAGES_CONTAINER"><div class="comp-2 rich-text">
<h1 class="font_0"><span style="color:#222">Handmade ceramics</span></h1>
<p class="font_8">Bowls, mugs and plates thrown by hand in Lyon.</p>
<p class="font_8">Fired at <strong>1280 °C</strong> for strength.</p>
<p class="font_8">Free shipping on orders over 80 €.</p>
<p class="font_8">Write to hello@shop.test or call +33 6 12 34 56 78.</p>
<p class="font_8">Visit us at 12 rue de la Paix, 75002 Paris.</p>
<p class="member">Hi Jane Doe</p>
</div>
<form id="contact"><label for="e">Your email</label><input id="e" name="email" placeholder="you@example.com">
<p class="help">We usually answer within a day.</p><textarea name="msg"></textarea><button type="submit">Send</button></form>
</main>
<script>
  // Builder-style SPA routing: content swapped in place, URL changed with pushState.
  const pages = { "/about": '<div class="comp-3"><h2>Our story</h2><p>Three potters in a former silk workshop.</p></div>' };
  const home = document.getElementById("PAGES_CONTAINER").innerHTML;
  const show = () => { document.getElementById("PAGES_CONTAINER").innerHTML = pages[location.pathname] ?? home; document.title = location.pathname === "/about" ? "About — Atelier" : "Ceramics — Atelier"; };
  document.addEventListener("click", (e) => { const a = e.target.closest("[data-route]"); if (!a) return; e.preventDefault(); history.pushState({}, "", a.getAttribute("href")); show(); });
  addEventListener("popstate", show);
  if (location.pathname !== "/") show();
</script>
</body></html>`);
  });
}

type Log = { url: string; body: string; cookie?: string }[];
function glot(status: () => object, log: Log) {
  return createServer(async (req, res) => {
    const u = new URL(req.url!, "http://x");
    log.push({ url: req.url!, body: await body(req), cookie: req.headers.cookie });
    const cors = { "access-control-allow-origin": "*" };
    const json = (v: unknown, code = 200) => res.writeHead(code, { "content-type": "application/json", ...cors }).end(JSON.stringify(v));
    if (u.pathname === "/widget/glot.js") return res.writeHead(200, { "content-type": "text/javascript", ...cors }).end(JS);
    if (u.pathname === "/edge/v1/site/status") return json(status());
    if (u.pathname === "/cdn/site/locales.json") return json({ source: "en", updated_at: null, locales: [{ code: "en", name: "English", dir: "ltr", coverage: 1 }, { code: "es", name: "Español", dir: "ltr", coverage: 0.6 }, { code: "ar", name: "العربية", dir: "rtl", coverage: 0 }] });
    if (u.pathname === "/edge/v1/site/es/tm") return json(TM);
    if (u.pathname === "/edge/v1/site/missing") return json({}, 202);
    json({}, 404);
  });
}

const posts = (log: Log) => log.filter((l) => l.url.includes("/missing")).map((l) => JSON.parse(l.body));
function assertNothingPersonal(log: Log) {
  const all = JSON.stringify(log);
  for (const x of PII) assert.ok(!all.includes(x), `glot received "${x}": ${all}`);
  assert.ok(log.every((l) => !l.cookie), "no cookie ever sent to glot");
}
/** Opens the shadow-DOM menu of the switcher and picks `name`. */
async function pick(page: any, name: string) {
  await page.locator("glot-locale-switcher").locator("[part~=trigger]").click();
  await page.locator("glot-locale-switcher").locator(`[role=option]:has-text("${name}")`).click();
}

test("website tag: switcher, in-place translation, SPA, discovery without personal data, memory, SEO bases", { skip: noBrowser }, async () => {
  const { chromium } = await import(PW);
  const b = await chromium.launch();
  const servers: Server[] = [];
  try {
    const log: Log = [];
    let status: Record<string, unknown> = { enabled: false, versions: { es: 3 }, selectors: { exclude: [".member"] }, website: { on: true, position: "bottom-left", locales: ["es"] }, hreflang: {} };
    const g = glot(() => status, log);
    const G = `http://127.0.0.1:${await listen(g)}`;
    const tag = `<script src="${G}/widget/glot.js" data-project="site" data-key="pk_test" data-server="${G}" async></script>`;
    const s = site(() => tag);
    const S = `http://localhost:${await listen(s)}`;
    servers.push(g, s);

    // ------------------------------------------------------------------ 1. source-language visitor
    const ctx = await b.newContext({ locale: "en-US" });
    await ctx.addCookies([{ name: "theme", value: "s3cr3t", url: S }]); // a cookie of the site: never glot's business
    const page = await ctx.newPage();
    const errors: string[] = [];
    page.on("pageerror", (e: Error) => errors.push(e.message));
    await page.goto(`${S}/?utm_source=news`);
    await page.locator("glot-locale-switcher").locator("[part~=trigger]").waitFor();
    assert.equal(await page.textContent("h1"), "Handmade ceramics");
    const box = await page.evaluate(() => { const r = document.querySelector("glot-locale-switcher")!.getBoundingClientRect(); return { left: r.left, bottom: innerHeight - r.bottom, fixed: getComputedStyle(document.querySelector("glot-locale-switcher")!.parentElement!).position }; });
    assert.ok(box.fixed === "fixed" && box.left < 40 && box.bottom < 40, `bottom-left corner: ${JSON.stringify(box)}`);
    const offered = await page.locator("glot-locale-switcher").locator("[role=option]").allTextContents();
    assert.equal(offered.length, 2, `offered = source + website.locales: ${offered}`);
    assert.ok(!log.some((l) => l.url.includes("/tm")), "no TM in the source language");

    // ------------------------------------------------------------------ 2. pick Spanish: in place, no reload
    await page.evaluate(() => ((window as any).__same = true));
    await pick(page, "Español");
    await page.waitForFunction(() => document.querySelector("h1")?.textContent === "Cerámica artesanal");
    assert.equal(await page.evaluate(() => (window as any).__same), true, "no reload");
    assert.equal(await page.evaluate(() => document.documentElement.lang), "es");
    assert.equal(await page.title(), "Cerámica — Atelier");
    assert.equal(await page.getAttribute("meta[name=description]", "content"), "Cerámica hecha para durar.");
    assert.equal(await page.innerHTML("main p:nth-of-type(2)"), "Cocidos a <strong>1280 °C</strong> para mayor resistencia.");
    assert.equal(await page.textContent("#contact button"), "Enviar");
    assert.equal(await page.textContent(".member"), "Hi Jane Doe"); // excluded selector: untouched
    await page.fill("#e", "jane.doe@example.com"); // what the visitor types never leaves
    await page.waitForTimeout(1800); // discovery debounce
    let p = posts(log);
    assert.equal(p.length, 1, JSON.stringify(p));
    assert.equal(p[0].mode, "website");
    assert.equal(p[0].key, "pk_test");
    assert.equal(p[0].locale, "es");
    assert.deepEqual(p[0].segments.map((x: { text: string }) => x.text).sort(), ["Free shipping on orders over {0} €.", "Your email"]);
    assert.ok(p[0].segments.every((x: { url: string; hash: string; text: string }) => x.url === "/" && x.hash === h(x.text)));

    // SPA route: translated as it renders, its new text reported with its own path.
    await page.click("nav a[href='/about']");
    await page.waitForFunction(() => document.querySelector("h2")?.textContent === "Nuestra historia");
    await page.waitForTimeout(1800);
    p = posts(log);
    assert.equal(p.length, 2, JSON.stringify(p));
    assert.deepEqual(p[1].segments.map((x: { text: string; url: string }) => [x.text, x.url]).sort(), [["About — Atelier", "/about"], ["Three potters in a former silk workshop.", "/about"]]);

    // ------------------------------------------------------------------ 3. remembered; back to the source
    await page.reload();
    await page.waitForFunction(() => document.querySelector("h2")?.textContent === "Nuestra historia");
    await pick(page, "English");
    await page.waitForFunction(() => document.querySelector("h2")?.textContent === "Our story");
    assert.equal(await page.evaluate(() => document.documentElement.lang), "en");
    await page.goto(`${S}/?lang=es`);
    await page.waitForFunction(() => document.querySelector("h1")?.textContent === "Cerámica artesanal");
    await page.waitForTimeout(1800); // its report lands before the next count
    assertNothingPersonal(log);
    assert.deepEqual(errors, []);
    await ctx.close();

    // ------------------------------------------------------------------ 4. signed-in visitor; tag off
    const before = posts(log).length;
    const c2 = await b.newContext({ locale: "es-ES" });
    await c2.addCookies([{ name: "member_session", value: "1", url: S }]);
    const p2 = await c2.newPage();
    await p2.goto(S);
    await p2.waitForFunction(() => document.querySelector("h1")?.textContent === "Cerámica artesanal"); // browser language
    await p2.waitForTimeout(1800);
    assert.equal(posts(log).length, before, "never discovered with a session cookie");
    await c2.close();

    status = { ...status, website: { on: false } };
    const c3 = await b.newContext({ locale: "es-ES" });
    const p3 = await c3.newPage();
    const tms = log.filter((l) => l.url.includes("/tm")).length;
    await p3.goto(S);
    await p3.waitForTimeout(800);
    assert.equal(await p3.textContent("h1"), "Handmade ceramics");
    assert.equal(await p3.locator("glot-locale-switcher").count(), 0);
    assert.equal(log.filter((l) => l.url.includes("/tm")).length, tms);
    await c3.close();

    // ------------------------------------------------------------------ 5. SEO-ready Edge subdomain
    status = { ...status, website: { on: true, position: "#SITE_HEADER .comp-1" }, hreflang: { en: S, es: "https://es.atelier.test" } };
    const c4 = await b.newContext({ locale: "en-US" });
    const p4 = await c4.newPage();
    await p4.route("https://es.atelier.test/**", (r: any) => r.fulfill({ contentType: "text/html", body: "<h1>es</h1>" }));
    await p4.goto(`${S}/?ref=x`);
    await p4.locator("#SITE_HEADER .comp-1 glot-locale-switcher").locator("[part~=trigger]").waitFor(); // inserted into the selector
    const links = () => p4.evaluate(() => [...document.querySelectorAll("link[rel=alternate][hreflang]")].map((l) => `${l.getAttribute("hreflang")} ${l.getAttribute("href")}`));
    assert.deepEqual(await links(), [`en ${S}/?ref=x`, "es https://es.atelier.test/?ref=x", `x-default ${S}/?ref=x`]);
    await p4.click("nav a[href='/about']");
    assert.deepEqual(await links(), [`en ${S}/about`, "es https://es.atelier.test/about", `x-default ${S}/about`]);
    await Promise.all([p4.waitForURL("https://es.atelier.test/about"), pick(p4, "Español")]);
    await c4.close();
    assertNothingPersonal(log);
  } finally {
    await b.close();
    for (const s of servers) s.close();
  }
});
