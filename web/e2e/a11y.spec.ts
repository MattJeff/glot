import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

// Accessibility gate for the dashboard: every main page, light and dark, must have
// zero serious/critical axe violations (WCAG 2.0–2.2 A/AA).
// Seeded by e2e/setup.ts (globalSetup).
const slug = process.env.E2E_SLUG!;

test.use({ storageState: "e2e/.auth.json" });

const PAGES = [
  "/en/app",
  `/en/app/${slug}`,
  `/en/app/${slug}/keys`,
  `/en/app/${slug}/glossary`,
  `/en/app/${slug}/jobs`,
  `/en/app/${slug}/settings`,
  `/en/app/${slug}/visual`,
  `/en/app/${slug}/feedback`,
  `/en/app/${slug}/preview`,
  `/en/app/${slug}/insights`,
  `/en/app/${slug}/experiments`,
  "/en/app/settings/account",
  "/en/app/settings/tokens",
  `/ar/app/${slug}`, // RTL
  `/ar/app/${slug}/keys`,
];

for (const scheme of ["light", "dark"] as const) {
  for (const path of PAGES) {
    test(`axe ${scheme} ${path}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
      await page.goto(path);
      await page.locator("h1").first().waitFor();
      await page.waitForLoadState("networkidle");
      const { violations } = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
        // The Next.js dev-tools badge only exists in `next dev`.
        .exclude("nextjs-portal")
        .analyze();
      const blocking = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      const report = blocking.map((v) => `${v.id} (${v.impact}): ${v.help}\n  ${v.nodes.slice(0, 3).map((n) => n.target.join(" ")).join("\n  ")}`);
      expect(report, report.join("\n")).toEqual([]);
    });
  }
}
