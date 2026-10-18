// Onboarding e2e (local): signup → create project → onboarding → real `glot push` with the CLI → celebration →
// steps ticked → axe-core (0 serious/critical) → screenshots light/dark/mobile/RTL.
//
//   BASE_URL=http://localhost:3400 API_URL=http://localhost:8810 GLOT_BIN=../target/integ/debug/glot \
//   AXE_PATH=/path/to/axe-core/axe.min.js SHOTS=/tmp/shots node e2e/onboarding.e2e.mjs
//
// The web app must proxy /api to API_URL (GLOT_API_URL) and the server must run with GLOT_PUBLIC_URL=API_URL.
// ponytail: plain node + Playwright from packages/visual-qa, no @playwright/test runner for one scenario.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import assert from "node:assert/strict";

const { chromium } = await import(new URL("../../packages/visual-qa/node_modules/playwright/index.mjs", import.meta.url));
const BASE = process.env.BASE_URL ?? "http://localhost:3400";
const API = process.env.API_URL ?? "http://localhost:8810";
const GLOT = process.env.GLOT_BIN ?? new URL("../../target/integ/debug/glot", import.meta.url).pathname;
const SHOTS = process.env.SHOTS ?? mkdtempSync(join(tmpdir(), "glot-onb-"));
const AXE = process.env.AXE_PATH;
mkdirSync(SHOTS, { recursive: true });

const browser = await chromium.launch();
const ctx = await browser.newContext({ locale: "en-US", viewport: { width: 1280, height: 900 } });
const page = await ctx.newPage();
const step = (s) => console.log(`• ${s}`);

async function axe(label) {
  if (!AXE) return console.log(`  (axe skipped: set AXE_PATH)`);
  await page.addScriptTag({ path: AXE });
  const { violations } = await page.evaluate(() =>
    window.axe.run(document, { runOnly: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] }),
  );
  const serious = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  for (const v of violations) console.log(`  axe ${label}: [${v.impact}] ${v.id} ×${v.nodes.length} ${v.nodes[0]?.target}`);
  assert.equal(serious.length, 0, `${label}: ${serious.map((v) => v.id).join(", ")}`);
}

// 1. Signup
const email = `onb-${Date.now()}@example.com`;
step(`signup ${email}`);
await page.goto(`${BASE}/signup`);
await page.getByLabel("Name").fill("Ada");
await page.getByLabel("Email").fill(email);
await page.getByLabel("Password").fill("correct horse battery");
await page.getByRole("button", { name: "Create account" }).click();
await page.waitForURL(/\/app$/);

// 2. Create project: name → source (detected) → targets
step("create project");
await page.getByRole("button", { name: /new project|create.*project/i }).first().click();
const slug = `onb-${Date.now().toString(36)}`;
await page.getByRole("textbox", { name: "Name" }).fill("Acme Web");
await page.getByRole("textbox", { name: "Slug" }).fill(slug);
await page.waitForTimeout(400);
await page.screenshot({ path: join(SHOTS, "create-1-name.png") });
await page.getByRole("button", { name: "Continue" }).click();
await assert.doesNotReject(page.getByText("Detected from your browser").waitFor());
assert.ok(await page.getByRole("radio", { name: /English/ }).first().isChecked(), "source detected from the browser");
await page.waitForTimeout(400);
await page.screenshot({ path: join(SHOTS, "create-2-source.png") });
await page.getByRole("button", { name: "Continue" }).click();
await page.getByRole("checkbox", { name: /French/ }).check({ force: true });
await page.getByRole("searchbox").fill("japa");
await page.getByRole("searchbox").press("Enter");
await page.getByText("2 languages selected").waitFor();
await page.waitForTimeout(400);
await page.screenshot({ path: join(SHOTS, "create-3-targets.png") });
await axe("create dialog");
await page.getByRole("button", { name: "Create project with 2 languages" }).click();
await page.waitForURL(new RegExp(`/app/${slug}/onboarding$`));

// 3. Onboarding: waiting state
step("onboarding waiting");
await page.getByText("Waiting for your first push…").waitFor();
await page.emulateMedia({ reducedMotion: "reduce" });
await page.screenshot({ path: join(SHOTS, "onboarding-waiting-light.png"), fullPage: true });
await axe("onboarding waiting light");
await page.emulateMedia({ reducedMotion: "no-preference" });

await page.getByText("Terminal", { exact: true }).click();
await page.getByRole("button", { name: "Generate my token" }).click();
const tokenBlock = page.locator("pre code", { hasText: /^glot_[0-9a-f]{64}$/ });
await tokenBlock.waitFor();
const token = (await tokenBlock.textContent()).trim();
const pushCmd = await page.locator("pre code", { hasText: "npx glot-i18n push" }).textContent();
assert.ok(pushCmd.includes(`project = "${slug}"`) && pushCmd.includes(token) && pushCmd.includes(API), "push command is pre-filled");
assert.ok(await page.evaluate(() => document.activeElement?.textContent?.includes("Token created")), "focus moved to the token");

// 4. Real `glot push` from a temp app
step("glot push");
const app = mkdtempSync(join(tmpdir(), "glot-onb-app-"));
mkdirSync(join(app, "messages"));
writeFileSync(
  join(app, "glot.toml"),
  `source_locale = "en"\ntarget_locales = ["fr", "ja"]\nfiles = ["messages/{locale}.json"]\n\n[llm]\nprovider = "mock"\nmodel = "mock"\n\n[server]\nurl = "${API}"\nproject = "${slug}"\n`,
);
writeFileSync(join(app, "messages/en.json"), JSON.stringify({ home: { title: "Welcome", cta: "Get started", save: "Save {count} files" } }));
writeFileSync(join(app, "messages/fr.json"), JSON.stringify({ home: { title: "Bienvenue", cta: "Commencer", save: "Enregistrer {count} fichiers" } }));
writeFileSync(join(app, "messages/ja.json"), JSON.stringify({ home: { title: "ようこそ" } }));
console.log(execFileSync(GLOT, ["push"], { cwd: app, env: { ...process.env, GLOT_TOKEN: token } }).toString().trim());

// 5. Live update → celebration → ticked steps
step("celebration");
await page.getByText("3 keys received").waitFor({ timeout: 10_000 });
await page.getByText("Milestone reached: First push").waitFor();
for (const s of ["Choose how you'll send your strings", "Push your first strings"]) {
  await page.getByRole("heading", { name: `${s} (done)` }).waitFor();
}
for (const s of ["Translate a first language", "Reach zero validation errors"]) {
  await page.getByText(`${s} (done)`).waitFor();
}
await page.getByText(/^Reached on /).first().waitFor();
await page.getByText("2 keys are missing in Japanese.").waitFor();
await page.waitForTimeout(1600); // confetti gone
await page.screenshot({ path: join(SHOTS, "onboarding-done-light.png"), fullPage: true });
await axe("onboarding done light");

for (const [name, opts] of [
  ["dark", { colorScheme: "dark" }],
  ["mobile", { colorScheme: "light", viewport: { width: 390, height: 844 } }],
  ["mobile-dark", { colorScheme: "dark", viewport: { width: 390, height: 844 } }],
]) {
  const c = await browser.newContext({ locale: "en-US", ...opts, storageState: await ctx.storageState() });
  const p = await c.newPage();
  await p.goto(`${BASE}/app/${slug}/onboarding`);
  await p.getByText("3 keys received").waitFor();
  await p.waitForTimeout(500); // entry animations
  await p.screenshot({ path: join(SHOTS, `onboarding-${name}.png`), fullPage: true });
  const width = await p.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  assert.ok(width <= 0, `${name}: no horizontal scroll (${width}px)`);
  await c.close();
}
await page.emulateMedia({ colorScheme: "dark" });
await axe("onboarding done dark");
await page.emulateMedia({ colorScheme: "light" });

step("overview + RTL");
await page.goto(`${BASE}/app/${slug}`);
await page.getByText("2 keys are missing in Japanese.").waitFor();
await page.screenshot({ path: join(SHOTS, "overview-light.png"), fullPage: true });
await axe("overview");
await page.goto(`${BASE}/ar/app/${slug}/onboarding`);
await page.locator("[role=status]").first().waitFor();
await page.waitForTimeout(500);
await page.screenshot({ path: join(SHOTS, "onboarding-ar.png"), fullPage: true });

await browser.close();
console.log(`✓ onboarding e2e passed — screenshots in ${SHOTS}`);
