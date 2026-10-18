import { chromium } from "../../../packages/visual-qa/node_modules/playwright/index.mjs";
import { readFileSync } from "node:fs";
const S = process.argv[2];
const img = (n) => `data:image/png;base64,${readFileSync(`${S}/${n}.png`).toString("base64")}`;
const card = (title, sub, n, tone) => `<figure><figcaption><b style="color:${tone}">${title}</b><span>${sub}</span></figcaption><img src="${img(n)}"></figure>`;
const html = (body) => `<!doctype html><meta charset=utf-8><style>
body{margin:0;padding:28px;background:#f6f7f9;font:15px/1.4 -apple-system,system-ui,sans-serif;color:#111;display:flex;gap:24px;align-items:flex-start}
figure{margin:0;background:#fff;border:1px solid #e3e5e8;border-radius:12px;padding:16px}
figcaption{display:flex;flex-direction:column;gap:2px;margin-bottom:12px}figcaption span{color:#667085;font-size:13px;font-family:ui-monospace,Menlo,monospace}
img{width:368px;display:block}
</style>${body}`;
const before = card("Before: de, w-28 button", "\"Zahlungserinnerung senden\" (overflows, truncated)", "fit-before", "#c2410c");
const after = card("After: glot fit, 1 iteration", "\"Mahnung\" (claude-haiku-5-5, $0.0002)", "fit-after", "#15803d");
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 900, height: 300 }, deviceScaleFactor: 2 });
await p.setContent(html(before + after));
const h = await p.evaluate(() => document.body.scrollHeight);
await p.setViewportSize({ width: 900, height: h });
await p.screenshot({ path: `${S}/fit-before-after.png` });
await b.close();
