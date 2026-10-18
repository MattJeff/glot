import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { anchor, json, member, state, widget } from "./env";

// The widget on the customer's site (packages/review). Locators pierce its open Shadow DOM.
const bar = (p: Page) => p.getByRole("toolbar", { name: /glot review/i });

async function openWithLink(page: Page, token: string, path = "/de", name = "Lena") {
  await page.goto(`${path}?glot_review=${token}`);
  await expect(bar(page)).toBeVisible();
  await expect(page).not.toHaveURL(/glot_review/); // token moved to sessionStorage
  const who = page.getByRole("textbox", { name: /name/i });
  if (await who.isVisible().catch(() => false)) {
    await who.fill(name);
    await who.press("Enter");
  }
}

async function annotate(page: Page, target: string, body: string) {
  await bar(page).getByRole("button", { name: "Comment" }).click();
  await page.locator(target).click();
  const dlg = page.getByRole("dialog", { name: /annotation/i });
  await dlg.getByRole("textbox", { name: "Comment" }).fill(body);
  await dlg.getByRole("button", { name: "Send" }).click();
  return dlg;
}

const pins = (p: Page) => p.getByRole("button", { name: /^Annotation \d+/ });

test.afterEach(async () => {
  // Leave the project clean for the next test (member can delete anything as owner).
  const m = await member();
  for (const a of (await json(await m.get("annotations?status=open,in_progress,resolved,wontfix&per_page=100"))).items) await m.delete(`annotations/${a.id}`);
});

test("normal visitors get nothing: no widget, no request to glot", async ({ page }) => {
  const calls: string[] = [];
  page.on("request", (r) => r.url().includes(":8832") && calls.push(r.url()));
  await page.goto("/de");
  await page.waitForLoadState("networkidle");
  expect(calls).toEqual([]);
  await expect(bar(page)).toHaveCount(0);
  expect(await page.evaluate(() => [...document.scripts].some((s) => s.src.includes("review.app")))).toBe(false);
});

test("comment link: create, edit, delete an annotation on the site", async ({ page }) => {
  await openWithLink(page, state().comment);
  await annotate(page, "#hero-cta", "Overflows the button in German");
  await expect(pins(page)).toHaveCount(1);

  await pins(page).first().click();
  const panel = page.getByRole("dialog", { name: /annotation/i });
  await expect(panel.getByText("Overflows the button in German")).toBeVisible();
  await panel.getByRole("button", { name: "Edit" }).first().click();
  await panel.getByRole("textbox").first().fill("Overflows: max ~20 characters");
  await panel.getByRole("button", { name: "Save" }).click();
  await expect(panel.getByText("Overflows: max ~20 characters")).toBeVisible();

  page.once("dialog", (d) => d.accept());
  await panel.getByRole("button", { name: "Delete" }).first().click();
  await expect(pins(page)).toHaveCount(0);
});

test("member resolves on the site; the pin leaves the open filter", async ({ page }) => {
  await openWithLink(page, state().member);
  await annotate(page, "#hero-title", "Title reads stiff");
  await pins(page).first().click();
  const panel = page.getByRole("dialog", { name: /annotation/i });
  await panel.getByRole("button", { name: "Resolve" }).click();
  await expect(panel.getByRole("button", { name: "Reopen" })).toBeVisible();
  const m = await member();
  expect((await json(await m.get("annotations?status=resolved"))).total).toBe(1);
});

test("read link: sees pins, cannot comment", async ({ page }) => {
  const w = await widget(state().comment);
  await json(await w.post("annotations", { data: { anchor: anchor(), body: "From another reviewer", author_name: "Lena" } }));
  await openWithLink(page, state().read);
  await expect(pins(page)).toHaveCount(1);
  await expect(bar(page).getByRole("button", { name: "Comment" })).toHaveCount(0);
});

test("re-anchoring: the pin follows its text after the layout changes, orphan when the text is gone", async ({ page }) => {
  const w = await widget(state().comment);
  // Stored with a stale selector and rect: the widget must find the element by key/text.
  await json(await w.post("annotations", {
    data: { anchor: anchor({ selector: "main > div:nth-of-type(9)", rect: { x: 5, y: 5, w: 10, h: 10 } }), body: "Moved", author_name: "Lena" },
  }));
  await json(await w.post("annotations", {
    data: { anchor: anchor({ selector: "#gone", text: "A sentence that is not on this page anymore", rect: undefined }), body: "Gone", author_name: "Lena" },
  }));
  await openWithLink(page, state().comment);
  const title = await page.locator("#hero-title").boundingBox();
  const pin = await pins(page).first().boundingBox();
  expect(pin && title && pin.y >= title.y - 24 && pin.y <= title.y + title.height + 24).toBe(true);
  // Layout shift: the pin follows.
  await page.evaluate(() => document.querySelector(".hero")!.prepend(Object.assign(document.createElement("div"), { style: "height:300px" })));
  await expect.poll(async () => (await pins(page).first().boundingBox())!.y).toBeGreaterThan(pin!.y + 200);
  await bar(page).getByRole("button", { name: "Annotations" }).click();
  await expect(page.getByText(/orphan/i)).toBeVisible();
});

test("realtime: an annotation from one browser appears in another", async ({ page, browser }) => {
  await openWithLink(page, state().comment, "/de", "Lena");
  const other = await (await browser.newContext({ viewport: { width: 1280, height: 860 } })).newPage();
  await openWithLink(other, state().member);
  await annotate(page, "#features-title", "Too long for a heading");
  await expect(pins(other)).toHaveCount(1, { timeout: 5000 });
  // And the status change travels back.
  const m = await member();
  const [a] = (await json(await m.get("annotations"))).items;
  await json(await m.patch(`annotations/${a.id}`, { data: { status: "resolved" } }));
  await expect(pins(page)).toHaveCount(0, { timeout: 5000 }); // default filter = open
});

test("rewording: Shorter, preview, apply; an invalid manual text is refused", async ({ page }) => {
  await openWithLink(page, state().member);
  await annotate(page, "#hero-cta", "Overflows");
  await pins(page).first().click();
  const panel = page.getByRole("dialog", { name: /annotation/i });
  await panel.getByRole("button", { name: "Reword" }).click();
  await panel.getByRole("button", { name: "Shorter" }).click();
  const proposal = panel.getByRole("textbox", { name: /proposed text/i });
  await expect(proposal).not.toHaveValue("");
  const proposed = await proposal.inputValue();
  expect(proposed.length).toBeLessThan("Jetzt kostenlos und unverbindlich testen".length);

  // Invalid text: unknown placeholder → validator error, Apply disabled or refused.
  await proposal.fill("Jetzt {gratis} testen");
  await expect(panel.getByText(/placeholder|invalid|\{gratis\}/i)).toBeVisible();
  const m = await member();
  expect((await json(await m.get("annotations"))).items[0].status).toBe("open");

  await proposal.fill(proposed);
  await panel.getByRole("button", { name: "Apply" }).click();
  await expect(panel.getByText("Applied")).toBeVisible();
  const [a] = (await json(await m.get("annotations?status=resolved"))).items;
  expect(a.translation.value).toBe(proposed);
  await json(await m.put(`text/hero.cta`, { data: { values: { de: "Jetzt kostenlos und unverbindlich testen" } } }));
});

test("accessibility: widget toolbar and annotation dialog have no serious axe violation, keyboard works", async ({ page }) => {
  await openWithLink(page, state().comment);
  await bar(page).getByRole("button", { name: "Comment" }).click();
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: /annotation/i })).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
  const serious = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(serious.map((v) => `${v.id} ${v.nodes[0]?.target}`)).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: /annotation/i })).toHaveCount(0);
});
