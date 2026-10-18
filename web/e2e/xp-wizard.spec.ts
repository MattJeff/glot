import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page, type Route } from "@playwright/test";

// /experiments/new end to end: keys → variants (live validation) → audience → goal → launch.
// The segmented-experiments API (/xp, contract §4) is mocked in-page so the flow is testable before the server ships it;
// projects, keys, auth and insights come from the real server seeded by e2e/setup.ts.
// XP_SHOTS=<dir> saves light/dark/mobile captures of every step.
const slug = process.env.E2E_SLUG!;
const SHOTS = process.env.XP_SHOTS;

test.use({ storageState: "e2e/.auth.json" });

type Arm = { arm: number; intent: string; approved: boolean; values: Record<string, Record<string, string>> };

function mockXp(page: Page) {
  const exp = {
    id: "00000000-0000-4000-8000-000000000001",
    name: "",
    status: "draft",
    keys: [] as { key: string; source: string; source_changed: boolean }[],
    locales: [] as string[],
    cells: [],
    fits: { "nav.home": { max_width_px: 120, max_lines: 1 } },
    arms: [{ arm: 0, intent: "official", approved: true, values: {} }] as Arm[],
  };
  const check = () => {
    const cells = exp.arms
      .filter((a) => a.arm > 0)
      .flatMap((a) =>
        exp.keys.flatMap(({ key }) =>
          exp.locales.map((locale) => {
            const value = a.values[locale]?.[key] ?? null;
            const issues = !value
              ? [{ code: "empty", severity: "error", message: "Value is empty" }]
              : value.includes("!!")
                ? [{ code: "too_wide", severity: "error", message: `Text is 164 px wide; the budget for ${key} is 120 px` }]
                : [];
            return { arm: a.arm, key, locale, value, ok: issues.length === 0, issues };
          }),
        ),
      );
    return { ready: cells.every((c) => c.ok), problems: [], cells };
  };
  return page.route(/\/api\/projects\/[^/]+\/xp(\/.*)?(\?.*)?$/, async (route: Route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace(/^.*\/xp/, "");
    const body = req.postDataJSON?.() ?? {};
    const json = (data: unknown, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(data) });
    if (path === "/settings") return json({ holdout_pct: 0, persistence: "none", holdout_id: null });
    if (path === "/estimate") {
      const cells = [...(body.cells ?? []).map((c: { name: string }, i: number) => ({ cell: c.name, visitors_per_day: 40 / (i + 1), days: 120 })), { cell: "*", visitors_per_day: 900, days: 9 }];
      return json({ cells, days: 9 });
    }
    if (path === "" || path === `/${exp.id}`) {
      if (req.method() !== "GET") {
        if (body.keys) exp.keys = body.keys.map((key: string) => ({ key, source: key, source_changed: false }));
        if (body.locales) exp.locales = body.locales;
        if (body.name) exp.name = body.name;
      }
      return json(exp, req.method() === "POST" && path === "" ? 201 : 200);
    }
    if (path === `/${exp.id}/check`) return json(check());
    if (path === `/${exp.id}/generate`) {
      const intents: string[] = body.intents;
      intents.forEach((intent, i) => {
        const values: Arm["values"] = {};
        for (const l of exp.locales) values[l] = Object.fromEntries(exp.keys.map(({ key }) => [key, `${intent} ${l} ${i}`]));
        exp.arms.push({ arm: exp.arms.length, intent, approved: true, values });
      });
      return json({ arms: exp.arms.slice(1), rejected: [], policy: [{ arm: 1, level: "warn", message: "“Thousands of teams” is not a declared fact: replaced by a neutral wording." }] });
    }
    const arm = path.match(/\/arms\/(\d+)$/);
    if (arm) {
      const n = Number(arm[1]);
      if (req.method() === "DELETE") {
        exp.arms = exp.arms.filter((a) => a.arm !== n);
        return route.fulfill({ status: 204 });
      }
      let a = exp.arms.find((x) => x.arm === n);
      if (!a) exp.arms.push((a = { arm: n, intent: body.intent ?? "custom", approved: true, values: {} }));
      if (body.intent) a.intent = body.intent;
      for (const [l, kv] of Object.entries((body.values ?? {}) as Arm["values"])) a.values[l] = { ...a.values[l], ...kv };
      return json(a);
    }
    if (path === `/${exp.id}/start`) return json({ ...exp, status: "running" });
    return route.fulfill({ status: 404, body: "{}" });
  });
}

async function axe(page: Page) {
  const { violations } = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .exclude("nextjs-portal")
    .analyze();
  const blocking = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(blocking.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`)).toEqual([]);
}

async function shots(page: Page, name: string) {
  await axe(page);
  if (!SHOTS) return;
  await page.screenshot({ path: `${SHOTS}/${name}.png`, fullPage: true });
}

for (const mode of ["light", "dark", "mobile"] as const) {
  test(`experiment wizard (${mode})`, async ({ page }) => {
    if (mode === "mobile") await page.setViewportSize({ width: 375, height: 812 });
    else await page.setViewportSize({ width: 1280, height: 900 });
    await page.emulateMedia({ colorScheme: mode === "dark" ? "dark" : "light", reducedMotion: "reduce" });
    await page.addInitScript(() => localStorage.clear());
    await mockXp(page);
    await page.goto(`/en/app/${slug}/experiments/new`);
    await expect(page.getByRole("heading", { level: 2, name: "What do you want to test?" })).toBeVisible();

    // 1. Keys: continuing without a key explains why; a group adds related keys in one click.
    await page.getByRole("button", { name: /Continue to Variants/ }).click();
    await expect(page.getByRole("alert").filter({ hasText: "Select at least one key" })).toBeVisible();
    await page.getByLabel("Search keys").fill("nav");
    await page.getByRole("button", { name: /nav\.\*/ }).click();
    await expect(page.getByRole("heading", { name: "2 keys selected" })).toBeVisible();
    await shots(page, `${mode}-1-keys`);
    await page.getByRole("button", { name: /Continue to Variants/ }).click();

    // 2. Variants: AI generation, then live validation with the validator's exact message.
    await expect(page.getByRole("heading", { level: 2, name: "Write the variants" })).toBeFocused();
    await page.getByRole("button", { name: /Generate 2 variants with AI/ }).click();
    await expect(page.getByRole("alert").filter({ hasText: "Some texts were held back" })).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: /All \d+ texts pass validation/ })).toBeVisible();
    const cell = page.getByLabel("Variant A, nav.home, French");
    await cell.fill("Accueil trop long!!");
    await expect(cell).toHaveAttribute("aria-invalid", "true");
    await expect(page.getByText("Text is 164 px wide; the budget for nav.home is 120 px").first()).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: "1 text needs a fix" })).toBeVisible();
    await shots(page, `${mode}-2-variants-error`);
    await page.getByRole("button", { name: /Continue to Audience/ }).click();
    await expect(page.getByRole("alert").filter({ hasText: "validation error" })).toBeVisible();
    await cell.fill("Accueil");
    await expect(page.getByRole("status").filter({ hasText: /All \d+ texts pass validation/ })).toBeVisible();
    await page.getByRole("button", { name: /Continue to Audience/ }).click();

    // 3. Audience: a preset segment, traffic estimate, fragmentation warning.
    await page.getByRole("button", { name: "Mobile visitors" }).click();
    await expect(page.getByText("Your traffic is split too thin")).toBeVisible();
    await shots(page, `${mode}-3-audience`);
    await page.getByRole("button", { name: /Continue to Goal/ }).click();

    // 4. Goal & launch (keyboard only).
    await page.getByRole("textbox", { name: "Event name" }).fill("signup");
    await page.getByRole("radio", { name: /Session cookie/ }).check();
    await shots(page, `${mode}-4-launch`);
    await page.getByRole("button", { name: "Launch experiment" }).focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/experiments\/00000000-0000-4000-8000-000000000001$/);
  });
}
