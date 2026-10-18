import { chromium } from "../../../packages/visual-qa/node_modules/playwright/index.mjs";
const S = process.argv[2];
const cursor = () => {
  addEventListener("DOMContentLoaded", () => {
    const c = document.createElement("div");
    c.style.cssText = "position:fixed;z-index:2147483647;pointer-events:none;width:18px;height:18px;border-radius:50%;background:rgba(37,99,235,.35);border:2px solid #2563eb;transform:translate(-50%,-50%);left:-50px;top:-50px;transition:transform .1s";
    document.documentElement.appendChild(c);
    addEventListener("mousemove", (e) => { c.style.left = e.clientX + "px"; c.style.top = e.clientY + "px"; }, true);
    addEventListener("mousedown", () => (c.style.transform = "translate(-50%,-50%) scale(.6)"), true);
    addEventListener("mouseup", () => (c.style.transform = "translate(-50%,-50%)"), true);
  });
};
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 760 }, recordVideo: { dir: `${S}/vid/ov`, size: { width: 1280, height: 760 } } });
await ctx.addInitScript(cursor);
const p = await ctx.newPage();
await p.goto("http://localhost:3255/invoices/inv_1042?glot_token=launchdemo", { waitUntil: "networkidle" });
await p.waitForTimeout(1200);
const glide = async (loc) => { const r = await loc.boundingBox(); await p.mouse.move(r.x + r.width / 2, r.y + r.height / 2, { steps: 25 }); };
const btn = p.locator("button", { hasText: "Send reminder" });
await glide(btn);
await p.waitForTimeout(400);
await p.keyboard.down("Alt");
await p.mouse.down(); await p.mouse.up();
await p.keyboard.up("Alt");
await p.waitForTimeout(3500);
await p.screenshot({ path: `${S}/overlay.png` });
// Human edit of the German label, validated and written to locales/de.json by glot dev.
// DE field position in the popover (see overlay.png); inputs live in a shadow root.
await p.mouse.move(1076, 435, { steps: 20 });
await p.mouse.down(); await p.mouse.up();
await p.keyboard.press("ControlOrMeta+A");
await p.keyboard.type("Erinnern", { delay: 90 });
await p.waitForTimeout(500);
await p.keyboard.press("Enter");
await p.waitForTimeout(2500);
await p.screenshot({ path: `${S}/overlay2.png` });
await ctx.close();
await b.close();
