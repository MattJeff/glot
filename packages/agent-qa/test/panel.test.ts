import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { capture, loadConfig, main } from "../src/panel.ts";

const cat: Record<string, Record<string, string>> = {
  en: { "nav.pricing": "Pricing", "hero.title": "Get paid faster", "hero.cta": "Start free trial", "hero.more": "See plans" },
  fr: { "nav.pricing": "Tarifs", "hero.title": "Soyez payé plus vite", "hero.cta": "Commencer l'essai gratuit", "hero.more": "Voir les offres" },
};

const page = (l: string) => {
  const t = (k: string) => cat[l]?.[k] ?? cat.en[k];
  return `<!doctype html><html lang="${l}"><head><title>Acme</title><meta name="description" content="Invoicing for teams"></head><body>
<nav><a href="/pricing">★ ${t("nav.pricing")}</a><a href="/hidden" style="display:none">Hidden</a></nav>
<main><h1>${t("hero.title")}</h1><button>${t("hero.cta")}</button><a href="/pricing">${t("hero.more")} →</a><a href="/pricing">★ ${t("nav.pricing")}</a></main>
</body></html>`;
};

test("panel capture: screenshot, text, CTAs of the page first, keys resolved", async () => {
  const dir = await mkdtemp(join(tmpdir(), "agent-qa-panel-"));
  for (const [l, c] of Object.entries(cat)) await writeFile(join(dir, `${l}.json`), JSON.stringify(c));
  const server = createServer((req, res) => {
    res.setHeader("content-type", "text/html; charset=utf-8");
    res.end(page(new URL(req.url!, "http://x").searchParams.get("lang") || "en"));
  });
  await new Promise<void>((r) => server.listen(0, r));
  const base = `http://localhost:${(server.address() as { port: number }).port}`;
  try {
    const pages = await capture({
      baseUrl: base, routes: ["/"], locales: ["en", "fr"], sourceLocale: "en", urlPattern: "{route}?lang={locale}",
      catalogs: join(dir, "{locale}.json"), viewport: "800x600", out: join(dir, "out"),
    });
    assert.equal(pages.length, 2);
    const fr = pages.find((p) => p.locale === "fr")!;
    assert.equal(fr.url, `${base}/?lang=fr`);
    assert.equal(fr.headline, "Soyez payé plus vite — Invoicing for teams");
    assert.match(fr.text, /Commencer l'essai gratuit/);
    // main first, deduped by label+href, hidden links skipped, icons ignored for key lookup.
    assert.deepEqual(fr.ctas.map((c) => [c.text, c.key]), [
      ["Commencer l'essai gratuit", "hero.cta"],
      ["Voir les offres →", "hero.more"],
      ["★ Tarifs", "nav.pricing"],
    ]);
    assert.ok((await stat(join(dir, "out", fr.screenshot))).size > 1000);

    // CLI entry: config routes + personas passed through to capture.json.
    await writeFile(join(dir, "glot.panel.yaml"), `product: Acme\nroutes: ["/"]\npersonas:\n  fr: { name: Inès, age: 63 }\n`);
    await main(["--config", join(dir, "glot.panel.yaml"), "--base-url", base, "--url-pattern", "{route}?lang={locale}",
      "--locales", "en,fr", "--catalogs", join(dir, "{locale}.json"), "--out", join(dir, "cli")]);
    const out = JSON.parse(await readFile(join(dir, "cli", "capture.json"), "utf8"));
    assert.deepEqual(out.config.routes, [{ route: "/" }]);
    assert.equal(out.config.personas.fr.name, "Inès");
    assert.equal(out.pages.length, 2);
  } finally {
    server.close();
    await rm(dir, { recursive: true, force: true });
  }
});

test("panel config: missing file is empty, bad routes are refused", async () => {
  const dir = await mkdtemp(join(tmpdir(), "agent-qa-panel-"));
  try {
    assert.deepEqual(await loadConfig(join(dir, "nope.yaml")), {});
    await writeFile(join(dir, "bad.yaml"), "routes: [pricing]\n");
    await assert.rejects(loadConfig(join(dir, "bad.yaml")), /starting with "\/"/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
