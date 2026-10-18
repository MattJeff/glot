import { chromium } from "../../../packages/visual-qa/node_modules/playwright/index.mjs";
const S = process.argv[2];
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1100, height: 680 }, recordVideo: { dir: `${S}/vid/term`, size: { width: 1100, height: 680 } } });
const p = await ctx.newPage();
await p.goto(`file://${S}/term.html`);
await p.waitForFunction(() => document.title === "done", null, { timeout: 120000 });
await p.waitForTimeout(2500);
await ctx.close(); await b.close();
