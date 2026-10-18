import { expect, test } from "@playwright/test";
import { anchor, json, member, state, widget } from "./env";

// Pixel 7 (412×915, touch). The widget must fit, pins must be tappable (≥ 24 px), no horizontal scroll.
test("mobile: toolbar fits, pin tappable, annotate by tap", async ({ page }) => {
  const w = await widget(state().comment);
  const seed = await json(await w.post("annotations", { data: { anchor: anchor({ viewport: { w: 412, h: 915 } }), body: "Seen on mobile", author_name: "Lena" } }));
  await page.goto(`/de?glot_review=${state().comment}`);
  const bar = page.getByRole("toolbar", { name: /glot review/i });
  await expect(bar).toBeVisible();
  const who = page.getByRole("textbox", { name: /name/i });
  if (await who.isVisible().catch(() => false)) { await who.fill("Lena"); await who.press("Enter"); }

  const box = (await bar.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(412);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);

  const pin = page.getByRole("button", { name: /^Annotation \d+/ }).first();
  const pb = (await pin.boundingBox())!;
  expect(Math.min(pb.width, pb.height)).toBeGreaterThanOrEqual(24);
  await pin.tap();
  await expect(page.getByRole("dialog", { name: /annotation/i }).getByText("Seen on mobile")).toBeVisible();
  await page.screenshot({ path: "test-results/mobile-review.png" });

  const m = await member();
  await m.delete(`annotations/${seed.id}`);
});
