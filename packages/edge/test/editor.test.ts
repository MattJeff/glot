// Dashboard preview / click-to-edit engine (widget/glot-editor.js) in a real Chromium: a builder-style page with
// hostile scripts is rendered translated in a sandboxed frame; the site's scripts never run, a click on a text
// reaches the dashboard with the right segment (hash, context, source, translation), and "never translate this
// block" proposes selectors the server understands, with how many elements each one matches.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { buildSync } from "esbuild";
import { hash16 } from "../src/segment.ts";

const PW = new URL("../../renderer/node_modules/playwright/index.mjs", import.meta.url).pathname;
const noBrowser = !existsSync(PW) && "playwright (packages/renderer) not installed";
const ROOT = new URL("..", import.meta.url).pathname;
const JS = buildSync({ entryPoints: [ROOT + "src/editor.global.ts"], bundle: true, write: false, format: "iife", target: "es2022" }).outputFiles[0].text;
const h = (t: string) => hash16(`en\u0000${t}`);

const SITE = `<!doctype html><html lang="en"><head><title>Atelier</title>
<meta http-equiv="refresh" content="0;url=https://evil.test/">
<script>parent.postMessage("pwned-inline", "*")</script><script src="https://evil.test/x.js"></script></head>
<body onload="parent.postMessage('pwned-onload','*')">
<div id="comp-kx3f9a2b" class="hero font_0"><h1 class="title">Handmade ceramics</h1></div>
<p class="font_8 promo">Free shipping on orders over 80 €.</p><p class="font_8">Not translated yet.</p>
<img alt="A blue bowl" onerror="parent.postMessage('pwned-onerror','*')" src="/missing.png">
<p class="member">Hi Jane Doe</p>
<a href="https://evil.test/" id="out">Shop</a></body></html>`;

test("editor engine: translated sandboxed preview, click → segment, exclusion selectors, site scripts never run", { skip: noBrowser }, async () => {
  const { chromium } = await import(PW);
  const s = createServer((req, res) => {
    if (req.url === "/glot-editor.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(JS);
    res.writeHead(200, { "content-type": "text/html" }).end(`<!doctype html><body><script src="/glot-editor.js"></script><iframe id="f" sandbox="allow-scripts" style="width:800px;height:600px"></iframe></body>`);
  });
  await new Promise<void>((ok) => s.listen(0, "127.0.0.1", ok));
  const b = await chromium.launch();
  try {
    const page = await b.newPage();
    await page.goto(`http://127.0.0.1:${(s.address() as { port: number }).port}/`);
    const tm = { v: 2, locale: "es", t: { [h("Handmade ceramics")]: "Cerámica artesanal", [h("Free shipping on orders over {0} €.")]: "Envío gratis a partir de {0} €.", [h("A blue bowl")]: "Un cuenco azul", [h("Shop")]: "Tienda" } };
    const info = await page.evaluate(({ html, tm }: { html: string; tm: unknown }) => {
      const w = window as any;
      w.msgs = [];
      addEventListener("message", (e) => w.msgs.push(e.data));
      w.out = w.glotEditor.render({ html, base: "https://shop.test/", path: "/", source: "en", tm, skip: [".member"] });
      (document.getElementById("f") as HTMLIFrameElement).srcdoc = w.out.srcdoc;
      return { segments: w.out.segments, translated: w.out.translated, srcdoc: w.out.srcdoc as string };
    }, { html: SITE, tm });
    assert.ok(!/evil\.test\/x\.js|pwned-inline|http-equiv="refresh"/.test(info.srcdoc), "site scripts and meta refresh removed");
    assert.ok(info.srcdoc.includes('<base href="https://shop.test/">'));
    assert.deepEqual(info.segments.map((x: { source: string; value: string | null }) => [x.source, x.value]), [
      ["Handmade ceramics", "Cerámica artesanal"],
      ["Free shipping on orders over {0} €.", "Envío gratis a partir de {0} €."],
      ["Not translated yet.", null],
      ["A blue bowl", "Un cuenco azul"],
      ["Shop", "Tienda"],
    ]); // .member (excluded selector) is not a segment
    assert.equal(info.translated, 4);

    const f = page.frameLocator("#f");
    await f.locator("h1").waitFor();
    assert.equal(await f.locator("h1").textContent(), "Cerámica artesanal");
    assert.equal(await f.locator("p.promo").textContent(), "Envío gratis a partir de 80 €.");
    assert.equal(await f.locator("img").getAttribute("alt"), "Un cuenco azul");
    assert.equal(await f.locator("html").getAttribute("lang"), "es");
    await f.locator("h1").click();
    await f.locator("#out").click(); // a link: relayed as a pick, never followed
    await page.waitForFunction(() => (window as any).msgs.filter((m: any) => m?.glot === "pick").length === 2);
    await page.waitForTimeout(300);
    const msgs = await page.evaluate(() => (window as any).msgs);
    assert.ok(!msgs.some((m: unknown) => typeof m === "string" && m.startsWith("pwned")), `a site script ran: ${JSON.stringify(msgs)}`);
    const picks = msgs.filter((m: any) => m?.glot === "pick");
    assert.equal(info.segments[Number(picks[0].segs)].hash, h("Handmade ceramics"));
    assert.equal(info.segments[Number(picks[0].segs)].context, "h1|/");
    assert.ok(picks[0].rect.w > 0 && picks[0].rect.h > 0);
    assert.equal(info.segments[Number(picks[1].segs)].source, "Shop");
    assert.equal(page.frames().length, 2, "the frame did not navigate away");
    assert.ok((await f.locator("h1").count()) === 1);

    // Exclusion: the text itself, then its block; a generated id (comp-kx3f9a2b) is not offered, its classes are.
    const sel = await page.evaluate(() => [(window as any).out.selectors(0), (window as any).out.selectors(1)]);
    assert.deepEqual(sel[0], [{ selector: "h1.title", count: 1 }, { selector: "div.hero.font_0", count: 1 }]);
    assert.deepEqual(sel[1], [{ selector: "p.font_8.promo", count: 1 }]);
  } finally {
    await b.close();
    s.close();
  }
});
