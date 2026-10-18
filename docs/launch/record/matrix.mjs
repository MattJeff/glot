import { chromium } from "../../../packages/visual-qa/node_modules/playwright/index.mjs";
import { readFileSync } from "node:fs";
const [S, record] = process.argv.slice(2);
const token = readFileSync(`${S}/cj`, "utf8").split("\n").find((l) => l.includes("glot_session")).split("\t")[6];
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 860 }, ...(record ? { recordVideo: { dir: `${S}/vid/matrix`, size: { width: 1440, height: 860 } } } : {}) });
await ctx.addCookies([{ name: "glot_session", value: token, domain: "localhost", path: "/" }]);
const p = await ctx.newPage();
await p.goto("http://localhost:3177/app/acme-invoices/preview", { waitUntil: "networkidle" });
await p.waitForTimeout(6000);
await p.screenshot({ path: `${S}/matrix.png` });
if (record) {
  await p.getByRole("button", { name: "Mobile" }).click();
  await p.waitForTimeout(4000);
  const route = p.locator("#matrix-route");
  await route.click();
  await route.fill("");
  await route.pressSequentially("/invoices/inv_1042", { delay: 60 });
  await route.press("Enter");
  await p.waitForTimeout(5000);
  await p.screenshot({ path: `${S}/matrix-mobile.png` });
  await p.getByRole("button", { name: "Desktop" }).click();
  await p.waitForTimeout(4000);
}
await ctx.close();
await b.close();
