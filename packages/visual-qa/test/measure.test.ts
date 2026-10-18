import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { existsSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright";
import { run } from "../src/qa.ts";
import { toSfnt } from "../src/fonts.ts";

const FIXTURES = join(import.meta.dirname, "../../../crates/core/tests/fixtures");
const FONT = join(FIXTURES, "Geist-Regular.ttf");
const FIXTURE = join(FIXTURES, "browser-widths.json");

const STRINGS = [
  "Save", "Cancel", "Send invite", "Mark as paid", "Create invoice", "Get started for free",
  "Rechnung erstellen", "Als bezahlt markieren", "Einladung senden", "Zahlungserinnerung",
  "Enregistrer les modifications", "Créer une facture", "Marquer comme payée", "Ça déborde, n'est-ce pas ?",
  "Guardar cambios", "Configuración de facturación", "WAVE AVATAR Typography", "1,234.56 € — 42%",
  "iiiiiiiiiiii", "MMMMMMMM", "The quick brown fox jumps over the lazy dog", "Überweisung fällig",
  "Ångström Øresund", "{count} items", "Invite teammates to your workspace", "AV To Ty Wa",
  "Löschen", "Archivieren", "Mitglied entfernen", "Abonnement kündigen",
  // Ligatures and kerning pairs (rustybuzz gets kern/liga explicitly).
  "Office waffle fluffy fjord", "AVATAR Yoga WAVE Typo", "Tâche effectuée",
];
const FONTS = [
  { size_px: 14, letter_spacing_px: 0 },
  { size_px: 16, letter_spacing_px: 0 },
  { size_px: 12, letter_spacing_px: 0.5 },
];

const serve = (html: Record<string, string>) =>
  new Promise<Server>((ok) => {
    const s = createServer(async (req, res) => {
      if (req.url === "/geist.ttf") { res.setHeader("content-type", "font/ttf"); res.end(await readFile(FONT)); return; }
      res.setHeader("content-type", "text/html; charset=utf-8");
      res.end(html[req.url ?? "/"] ?? "");
    }).listen(0, () => ok(s));
  });
const port = (s: Server) => (s.address() as { port: number }).port;
const FACE = `@font-face { font-family: Geist; src: url(/geist.ttf); } body, button { font-family: Geist; margin: 0 }`;

test("browser widths of 30 strings (fixture for glot-core's Rust measure)", async () => {
  const browser = await chromium.launch();
  const server = await serve({ "/": `<!doctype html><style>${FACE}</style><body>x</body>` });
  try {
    const page = await browser.newPage();
    await page.goto(`http://localhost:${port(server)}/`);
    const cases = [];
    for (const f of FONTS) {
      const widths = await page.evaluate(async ({ strings, f }) => {
        await document.fonts.load(`${f.size_px}px Geist`);
        return strings.map((t) => {
          const s = document.createElement("span");
          s.style.cssText = `font: 400 ${f.size_px}px Geist; letter-spacing: ${f.letter_spacing_px}px; white-space: pre`;
          s.textContent = t;
          document.body.append(s);
          const w = s.getBoundingClientRect().width;
          s.remove();
          return w;
        });
      }, { strings: STRINGS, f });
      for (const [i, text] of STRINGS.entries()) {
        cases.push({ text, px: Math.round(widths[i] * 100) / 100, font: { family: "Geist", postscript: "Geist-Regular", weight: 400, ...f } });
      }
    }
    if (process.env.GLOT_UPDATE_FIXTURE === "1" || !existsSync(FIXTURE)) {
      await writeFile(FIXTURE, JSON.stringify({ generated_by: "packages/visual-qa/test/measure.test.ts", cases }, null, 2) + "\n");
    } else if (process.platform === "darwin") {
      // The committed fixture was rendered by macOS Chromium; Linux font hinting snaps advances
      // to whole pixels (≈5% off on short strings), so only compare on the platform that made it.
      const stored = JSON.parse(await readFile(FIXTURE, "utf8")).cases as typeof cases;
      for (const [i, c] of cases.entries()) assert.ok(Math.abs(c.px - stored[i].px) < 0.5, `${c.text}: ${c.px} vs ${stored[i].px}`);
    }
  } finally {
    server.close();
    await browser.close();
  }
});

test("--measure: box budgets per key, real font names, webfonts saved as sfnt", async () => {
  const html = `<!doctype html><html lang="en"><style>${FACE}</style><body>
    <button style="width:112px;padding:0 8px;border:0;white-space:nowrap;overflow:hidden;font-size:14px">Mark as paid</button>
    <p style="width:200px;-webkit-line-clamp:2;display:-webkit-box;-webkit-box-orient:vertical;overflow:hidden;font-size:16px">Invite teammates to your workspace</p>
    <span style="display:inline-block;height:20px;width:120px;line-height:20px;overflow:hidden">Send invite</span>
    <div style="width:300px">Free text that wraps in a paragraph</div>
    <span style="position:absolute;white-space:nowrap">Grows with its text</span>
  </body></html>`;
  const server = await serve({ "/": html });
  const dir = await mkdtemp(join(tmpdir(), "glot-measure-"));
  await writeFile(join(dir, "en.json"), JSON.stringify({
    paid: "Mark as paid", invite_team: "Invite teammates to your workspace", send: "Send invite",
    free: "Free text that wraps in a paragraph", grows: "Grows with its text",
  }));
  try {
    const r = await run({
      baseUrl: `http://localhost:${port(server)}`, routes: ["/"], locales: [], sourceLocale: "en",
      urlPattern: "/{locale}{route}", viewports: ["800x600"], out: dir, suggest: false,
      catalogs: join(dir, "{locale}.json"), measure: true, fontsDir: join(dir, "fonts"),
    });
    const l = r.layout!;
    assert.equal(l.paid.max_width_px, 96);
    assert.equal(l.paid.max_lines, 1);
    assert.equal(l.paid.font.postscript, "Geist-Regular");
    assert.equal(l.paid.font.size_px, 14);
    assert.ok(l.paid.text_px! > 50 && l.paid.text_px! < 96, String(l.paid.text_px));
    assert.equal(l.invite_team.max_lines, 2);
    assert.equal(l.send.max_lines, 1);
    assert.equal(l.free.max_lines, null);
    assert.equal(l.free.max_width_px, 300);
    assert.equal(l.grows, undefined, "a box that grows with its text has no budget");
    const files = await readdir(join(dir, "fonts"));
    assert.equal(files.length, 1);
    assert.ok(files[0].endsWith(".ttf"));
  } finally {
    server.close();
  }
});

test("woff2 → sfnt keeps metrics tables", async (t) => {
  const root = join(import.meta.dirname, "../../../web/node_modules/.pnpm");
  const next = existsSync(root) ? readdirSync(root).find((d) => d.startsWith("next@")) : undefined;
  const woff2 = next && join(root, next, "node_modules/next/dist/next-devtools/server/font/geist-latin.woff2");
  if (!woff2 || !existsSync(woff2)) return t.skip("no woff2 sample installed");
  const sfnt = toSfnt(await readFile(woff2))!;
  const tags = Array.from({ length: sfnt.readUInt16BE(4) }, (_, i) => sfnt.toString("latin1", 12 + 16 * i, 16 + 16 * i));
  for (const tag of ["cmap", "head", "hhea", "hmtx", "maxp", "name"]) assert.ok(tags.includes(tag), `${tag} in ${tags}`);
  assert.ok(!tags.includes("glyf"));
});

test("--measure: dev key markers split two identical \"Save\" into two budgets; per-occurrence detail, tightest wins", async () => {
  const { encodeKey } = await import("../../runtime/src/dev.ts");
  const btn = (w: number, key: string) =>
    `<button style="width:${w}px;padding:0;border:0;white-space:nowrap;overflow:hidden;font-size:14px">${encodeKey(key)}Save</button>`;
  const html = `<!doctype html><html lang="en"><style>${FACE}</style><body>
    ${btn(60, "toolbar.save")} ${btn(200, "dialog.save")}
    ${btn(90, "common.cancel")} ${btn(150, "common.cancel")}
    <button style="width:70px;padding:0;border:0;white-space:nowrap;overflow:hidden;font-size:14px">Save</button>
  </body></html>`;
  const server = await serve({ "/": html });
  const dir = await mkdtemp(join(tmpdir(), "glot-measure-"));
  await writeFile(join(dir, "en.json"), JSON.stringify({
    toolbar: { save: "Save" }, dialog: { save: "Save" }, common: { cancel: "Save" },
  }));
  try {
    const r = await run({
      baseUrl: `http://localhost:${port(server)}`, routes: ["/"], locales: [], sourceLocale: "en",
      urlPattern: "/{locale}{route}", viewports: ["800x600"], out: dir, suggest: false,
      catalogs: join(dir, "{locale}.json"), measure: true, fontsDir: join(dir, "fonts"),
    });
    const l = r.layout!;
    assert.equal(l["toolbar.save"].max_width_px, 60);
    assert.equal(l["dialog.save"].max_width_px, 200);
    assert.equal(l["toolbar.save"].text, "Save", "markers are stripped before measuring");
    assert.deepEqual(l["common.cancel"].occurrences!.map((o) => [o.max_width_px, o.by]), [[90, "marker"], [150, "marker"]]);
    assert.equal(l["common.cancel"].max_width_px, 90, "tightest occurrence wins");
    // The unmarked "Save" falls back to text matching (first key with that text).
    const byText = Object.values(l).flatMap((f) => f.occurrences!).filter((o) => o.by === "text");
    assert.deepEqual(byText.map((o) => o.max_width_px), [70]);
  } finally {
    server.close();
  }
});

test("browser line counts of Japanese with punctuation (kinsoku fixture for glot-core, macOS)", async (t) => {
  if (process.platform !== "darwin") return t.skip("Hiragino is macOS-only");
  const TEXTS = [
    "設定を保存しました。変更は次回のログインから有効になります。",
    "「請求書」を作成、送信、そして支払いを確認します。",
    "メンバーを招待する（最大五人）、または後で追加する。",
    "ファイルが見つかりません！もう一度お試しください？",
    "保存、キャンセル、削除。",
  ];
  const font = { family: "Hiragino Sans", postscript: "HiraginoSans-W3", size_px: 16, weight: 400, letter_spacing_px: 0 };
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.setContent(`<!doctype html><html lang="ja"><body style="margin:0"></body></html>`);
    const cases = await page.evaluate(({ texts, font }) => {
      const out = [];
      for (const text of texts) {
        for (let width = 48; width <= 320; width += 13) {
          const d = document.createElement("div");
          d.style.cssText = `width:${width}px;font:${font.weight} ${font.size_px}px "${font.family}";line-height:20px`;
          d.textContent = text;
          document.body.append(d);
          const g = document.createRange();
          g.selectNodeContents(d);
          const lines = new Set(Array.from(g.getClientRects()).map((r) => Math.round(r.top))).size;
          d.remove();
          out.push({ text, width, lines, font });
        }
      }
      return out;
    }, { texts: TEXTS, font });
    const file = join(FIXTURES, "browser-lines-ja.json");
    if (process.env.GLOT_UPDATE_FIXTURE === "1" || !existsSync(file)) await writeFile(file, JSON.stringify({ generated_by: "packages/visual-qa/test/measure.test.ts", cases }, null, 2) + "\n");
  } finally {
    await browser.close();
  }
});
