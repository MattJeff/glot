import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

// Segmented experiment results (list, detail, preview by segment). The /xp API is mocked with a realistic
// payload so the page is checked independently of the stats engine; auth and project come from e2e/setup.ts.
const slug = process.env.E2E_SLUG!;
const shots = process.env.XP_SHOTS; // optional: directory for light/dark/mobile captures
test.use({ storageState: "e2e/.auth.json" });

const ID = "7d1c0a52-0000-4000-8000-000000000001";
const DACH = "5b0e2c11-0000-4000-8000-0000000000aa";
const MOBILE = "5b0e2c11-0000-4000-8000-0000000000bb";
const arm = (mean: number, p_best: number, lift: number, lo: number, hi: number, views: number) => ({
  mean, p_best, p_beats_control: p_best, lift, lift_lo: lo, lift_hi: hi, weight: 1 / 3, views, conversions: Math.round(mean * views),
});
const experiment = (status = "running") => ({
  id: ID, name: "Homepage hero", kind: "copy", status,
  keys: [{ key: "home.hero.title", source: "Ship every language", source_changed: false }, { key: "home.hero.cta", source: "Start free", source_changed: false }],
  locales: ["en", "fr", "de"],
  cells: [{ id: DACH, name: "DACH", rule: { all: [{ attr: "country", in: ["DE", "AT", "CH"] }] } }, { id: MOBILE, name: "Mobile search", rule: { all: [{ attr: "device", eq: "mobile" }] } }],
  allocation: "segmented", weights: null, traffic_pct: 100, min_n: 500, goal: "signup", metric: "conversion", currency: null,
  guardrails: [], facts: ["2,000 teams"], fits: {}, review: false,
  arms: [
    { arm: 0, intent: "official", approved: true, values: {} },
    { arm: 1, intent: "Social proof", approved: true, values: { en: { "home.hero.title": "Trusted by 2,000 teams", "home.hero.cta": "Join them" }, fr: { "home.hero.title": "Adopté par 2 000 équipes", "home.hero.cta": "Rejoignez-les" }, de: { "home.hero.title": "2.000 Teams vertrauen uns", "home.hero.cta": "Jetzt mitmachen" } } },
    { arm: 2, intent: "Direct benefit", approved: true, values: { en: { "home.hero.title": "Translate in minutes", "home.hero.cta": "Try it" }, fr: { "home.hero.title": "Traduisez en minutes", "home.hero.cta": "Essayer" }, de: { "home.hero.title": "In Minuten übersetzt", "home.hero.cta": "Testen" } } },
  ],
  state: [
    { cell: DACH, weights: [0.1, 0.8, 0.1], looks: 3, winner: 1, personalized: false },
    { cell: MOBILE, weights: [0.34, 0.33, 0.33], looks: 1, winner: null, personalized: false },
    { cell: "*", weights: [0.34, 0.33, 0.33], looks: 2, winner: null, personalized: false },
  ],
  created_at: "2026-09-30T08:00:00Z", started_at: "2026-10-01T08:00:00Z", ended_at: null,
});
const results = {
  look: 3, threshold: 0.99, holdout_srm_p: null, paused: false, everywhere: null, segments: [[DACH, 1]],
  global: { name: "global", views: 15000, arms: [], winner: null, p_specific: null, srm_p: 0.4 },
  cells: [
    { name: DACH, views: 4200, winner: 1, p_specific: 0.97, srm_p: 0.52, arms: [arm(0.05, 0.01, 0, 0, 0, 1400), arm(0.059, 0.97, 0.18, 0.06, 0.31, 1400), arm(0.051, 0.02, 0.02, -0.09, 0.13, 1400)] },
    { name: MOBILE, views: 900, winner: null, p_specific: null, srm_p: 0.0002, arms: [arm(0.03, 0.3, 0, 0, 0, 380), arm(0.031, 0.4, 0.03, -0.2, 0.3, 260), arm(0.029, 0.3, -0.03, -0.25, 0.22, 260)] },
    { name: "*", views: 9900, winner: null, p_specific: null, srm_p: 0.61, arms: [arm(0.04, 0.2, 0, 0, 0, 3300), arm(0.042, 0.62, 0.05, -0.04, 0.14, 3300), arm(0.04, 0.18, -0.01, -0.1, 0.08, 3300)] },
  ],
};

async function mock(page: Page) {
  let status = "running";
  await page.route(/\/api\/projects\/[^/]+\/xp(\/.*)?(\?.*)?$/, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace(/^.*\/xp/, "");
    if (route.request().method() === "POST" && /\/(pause|resume|stop|promote)$/.test(path)) {
      status = { pause: "paused", resume: "running", stop: "stopped", promote: "personalized" }[path.split("/").pop()!]!;
      return route.fulfill({ json: experiment(status) });
    }
    if (path === "" || path === "/") return route.fulfill({ json: [experiment(status)] });
    if (path.endsWith("/results")) return route.fulfill({ json: results });
    if (path === `/${ID}`) return route.fulfill({ json: experiment(status) });
    return route.fulfill({ status: 404, json: { error: { code: "not_found", message: "not found" } } });
  });
}

async function axe(page: Page) {
  const { violations } = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .exclude("nextjs-portal")
    .analyze();
  const blocking = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(blocking.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
}

for (const scheme of ["light", "dark"] as const) {
  test(`results page reads as a verdict, ${scheme}`, async ({ page }) => {
    await mock(page);
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.goto(`/en/app/${slug}/experiments/${ID}`);
    const verdict = page.getByRole("status").filter({ hasText: "Social proof" });
    await expect(verdict).toContainText("converts +18% better in DACH (97% probability)");
    await expect(verdict).toContainText("No winner yet elsewhere.");
    await expect(page.getByText("Traffic split looks wrong")).toBeVisible();
    await expect(page.getByRole("table", { name: "Results for DACH" })).toBeVisible();
    await page.waitForLoadState("networkidle");
    await axe(page);
    if (shots) await page.screenshot({ path: `${shots}/detail-${scheme}.png`, fullPage: true });
  });
}

test("promotion asks first, keyboard only, and pause reports back", async ({ page }) => {
  await mock(page);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`/en/app/${slug}/experiments/${ID}`);
  const promote = page.getByRole("button", { name: "Promote for this segment" });
  await promote.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Show “Social proof” to DACH permanently?");
  await expect(dialog).toContainText("all 3 languages");
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(promote).toBeFocused();

  await page.getByRole("button", { name: "Pause" }).click();
  await expect(page.getByText("Experiment paused")).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
});

test("list shows the segmented experiment, its verdict and milestones", async ({ page }) => {
  await mock(page);
  await page.goto(`/en/app/${slug}/experiments`);
  const row = page.getByRole("link", { name: /Homepage hero/ });
  await expect(row).toContainText("“Social proof” wins in DACH.");
  await expect(page.getByRole("list", { name: "Experiment milestones" })).toContainText("First winner");
  await page.waitForLoadState("networkidle");
  await axe(page);
  if (shots) await page.screenshot({ path: `${shots}/list-light.png`, fullPage: true });
  await row.click();
  await expect(page).toHaveURL(new RegExp(`/experiments/${ID}$`));
});

test("preview by segment: one row per wording, language by language", async ({ page }) => {
  await mock(page);
  await page.goto(`/en/app/${slug}/preview?xp=${ID}&cell=${DACH}`);
  await expect(page.getByRole("heading", { name: "Preview by segment" })).toBeVisible();
  await expect(page.getByText("Served to this segment").first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await axe(page);
  if (shots) await page.screenshot({ path: `${shots}/preview-light.png`, fullPage: true });
});

test("preview by segment with a preview URL: real pages, arm forced, not counted", async ({ page, baseURL }) => {
  await mock(page);
  await page.route(/\/api\/projects\/[^/]+\/preview$/, (route) =>
    route.fulfill({ json: { base_url: baseURL, url_pattern: "/{locale}{route}", routes: ["/login"] } }),
  );
  await page.goto(`/en/app/${slug}/preview?xp=${ID}&cell=${DACH}`);
  const frame = page.getByTitle("Page preview in French").first();
  await expect(frame).toHaveAttribute("src", new RegExp(`/fr/?\\?glot_xp=${ID}\\.0$`));
  await expect(page.getByTitle("Page preview in French")).toHaveCount(3); // one per approved arm
  await page.waitForLoadState("networkidle");
  await axe(page);
  if (shots) {
    await page.emulateMedia({ colorScheme: "dark" });
    await page.screenshot({ path: `${shots}/preview-frames-dark.png` });
  }
});

test("mobile: no horizontal page scroll", async ({ page }) => {
  await mock(page);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto(`/en/app/${slug}/experiments/${ID}`);
  await expect(page.getByRole("heading", { level: 1, name: "Homepage hero" })).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  if (shots) {
    await page.screenshot({ path: `${shots}/detail-mobile.png`, fullPage: true });
    await page.emulateMedia({ colorScheme: "dark" });
    await page.screenshot({ path: `${shots}/detail-mobile-dark.png`, fullPage: true });
  }
});
