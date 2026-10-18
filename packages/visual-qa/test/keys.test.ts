import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, mkdir, writeFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildIndex, match } from "../src/keys.ts";
import { run } from "../src/qa.ts";

test("reverse index: exact, placeholders, plurals, tags, case and whitespace", () => {
  const idx = buildIndex({
    "nav.home": "Home",
    "hello": "Hello <b>{name}</b>!",
    "files": "{count, plural, =0 {No files} one {# file} other {# files}} in {folder}",
    "cta": "Get   started",
    "any": "{name}",
    "seats": "{count, plural, one {# seat} other {# seats}}",
  });
  assert.equal(match(idx, "  HOME "), "nav.home");
  assert.equal(match(idx, "Hello Ada Lovelace!"), "hello");
  assert.equal(match(idx, "3 files in Docs"), "files");
  assert.equal(match(idx, "No files in Docs"), "files");
  assert.equal(match(idx, "Get\nstarted"), "cta");
  assert.equal(match(idx, "2 seats"), "seats");
  assert.equal(match(idx, "Something else"), undefined);
});

test("issue is attached to its key from catalogs alone (no data-glot-key)", async () => {
  const html = (lang: string, title: string, cta: string) => `<!doctype html><html lang="${lang}"><body style="font-family:sans-serif">
    <section><h1>${title}</h1><button style="width:120px;white-space:nowrap;overflow:hidden">${cta}</button></section></body></html>`;
  const pages: Record<string, string> = {
    "/": html("en", "Welcome, Ada", "Start free trial"),
    "/de": html("de", "Willkommen, Ada", "Kostenlose Testversion starten"),
  };
  const server = createServer((req, res) => {
    const body = pages[req.url!];
    res.writeHead(body ? 200 : 404, { "content-type": "text/html; charset=utf-8" }).end(body ?? "");
  });
  await new Promise<void>((r) => server.listen(0, r));
  const dir = await mkdtemp(join(tmpdir(), "glot-vqa-keys-"));
  await mkdir(join(dir, "messages"));
  await writeFile(join(dir, "messages/en.json"), JSON.stringify({ home: { title: "Welcome, {name}", cta: "Start free trial" } }));
  await writeFile(join(dir, "messages/de.json"), JSON.stringify({ home: { title: "Willkommen, {name}", cta: "Kostenlose Testversion starten" } }));
  try {
    const report = await run({
      baseUrl: `http://localhost:${(server.address() as { port: number }).port}`,
      routes: ["/"], locales: ["de"], sourceLocale: "en", urlPattern: "/{locale}{route}",
      viewports: ["1280x800"], out: join(dir, "out"), suggest: false,
      catalogs: join(dir, "messages/{locale}.json"), map: "*",
    });
    const o = report.issues.find((i) => i.kind === "overflow");
    assert.ok(o, JSON.stringify(report.issues));
    assert.equal(o.key, "home.cta");
    assert.equal(o.fit!.width <= 120, true);
    assert.ok(o.fit!.need_width > o.fit!.width && o.fit!.font);
    assert.ok(o.context_crop && (await stat(join(dir, "out", o.context_crop))).size > 0);
    const keys = report.keys!.map((k) => `${k.locale}:${k.key}`).sort();
    assert.deepEqual(keys, ["de:home.cta", "de:home.title", "en:home.cta", "en:home.title"]);
  } finally {
    server.close();
  }
});
