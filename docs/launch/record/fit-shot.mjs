import { chromium } from "../../../packages/visual-qa/node_modules/playwright/index.mjs";
const [S, name] = process.argv.slice(2);
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
await p.goto("http://localhost:3255/de/invoices/inv_1042", { waitUntil: "networkidle" });
const box = await p.locator("button", { hasText: /Archiv/ }).first().evaluate((el) => {
  const r = el.parentElement.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height };
});
await p.screenshot({ path: `${S}/${name}.png`, clip: { x: box.x - 24, y: box.y - 20, width: box.width + 48, height: box.height + 40 } });
await b.close();
