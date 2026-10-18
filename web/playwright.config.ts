import { defineConfig } from "@playwright/test";

// Runs against a live dashboard + glot-server (see e2e/a11y.spec.ts):
//   GLOT_API_URL=http://localhost:8812 pnpm dev -p 3420   then   E2E_BASE_URL=http://localhost:3420 pnpm test:e2e
// e2e/responsive.spec.ts also needs E2E_API_URL=http://localhost:8812 (OAuth consent route); RESPONSIVE_FULL=1 for all widths/themes.
export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/setup.ts",
  outputDir: "e2e/.results",
  timeout: 60_000,
  fullyParallel: true,
  reporter: [["list"]],
  use: { baseURL: process.env.E2E_BASE_URL ?? "http://localhost:3100", browserName: "chromium" },
});
