import { defineConfig, devices } from "@playwright/test";

// Runs against a live stack (see README): glot-server :8832, dashboard :3520, this site :3530.
export default defineConfig({
  testDir: "test",
  globalSetup: "./test/setup.ts",
  outputDir: "test-results",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  workers: 1, // one shared project; tests create their own annotations
  reporter: [["list"]],
  use: { baseURL: process.env.SITE_URL ?? "http://localhost:3530", browserName: "chromium", trace: "retain-on-failure" },
  projects: [
    { name: "desktop", use: { viewport: { width: 1280, height: 860 } }, testIgnore: /mobile/ },
    { name: "mobile", use: { ...devices["Pixel 7"] }, testMatch: /mobile/ },
  ],
});
