import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildUrl, run } from "../src/qa.ts";

const page = (lang: string, cta: string, dir = "ltr") => `<!doctype html>
<html lang="${lang}" dir="${dir}"><body style="font-family: sans-serif">
  <h1>Welcome back</h1>
  <button data-glot-key="auth.login.cta" style="width:120px;white-space:nowrap;overflow:hidden">${cta}</button>
</body></html>`;

const pages: Record<string, string> = {
  "/login": page("en", "Sign in"),
  "/de/login": page("de", "Jetzt kostenlos anmelden und loslegen"),
  "/ar/login": page("ar", "تسجيل الدخول"), // dir missing on purpose
};

test("buildUrl: source locale has no prefix", () => {
  const o = { baseUrl: "http://x/", urlPattern: "/{locale}{route}", sourceLocale: "en" };
  assert.equal(buildUrl(o, "en", "/"), "http://x/");
  assert.equal(buildUrl(o, "fr", "/"), "http://x/fr");
  assert.equal(buildUrl(o, "fr", "/login"), "http://x/fr/login");
});

test("detects overflowing German button, untranslated text and missing rtl", async () => {
  const server = createServer((req, res) => {
    const body = pages[req.url!];
    res.writeHead(body ? 200 : 404, { "content-type": "text/html; charset=utf-8" }).end(body ?? "");
  });
  await new Promise<void>((r) => server.listen(0, r));
  const port = (server.address() as { port: number }).port;
  const out = await mkdtemp(join(tmpdir(), "glot-vqa-"));
  try {
    const report = await run({
      baseUrl: `http://localhost:${port}`,
      routes: ["/login"],
      locales: ["de", "ar"],
      sourceLocale: "en",
      urlPattern: "/{locale}{route}",
      viewports: ["1280x800"],
      out,
      suggest: false,
    });
    const de = report.issues.filter((i) => i.locale === "de");
    const overflow = de.find((i) => i.kind === "overflow");
    assert.ok(overflow, JSON.stringify(report.issues, null, 2));
    assert.equal(overflow.key, "auth.login.cta");
    assert.ok(overflow.crop && (await stat(join(out, overflow.crop))).size > 0);
    assert.ok(de.some((i) => i.kind === "untranslated" && i.text === "Welcome back"));
    assert.ok(report.issues.some((i) => i.locale === "ar" && i.kind === "rtl"));
    assert.ok(!report.issues.some((i) => i.locale === "ar" && i.kind === "overflow"));
    assert.match(await readFile(join(out, "report.md"), "utf8"), /\| `de` \| 1 \|/);
  } finally {
    server.close();
  }
});
