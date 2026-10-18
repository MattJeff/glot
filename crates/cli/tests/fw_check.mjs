// Browser check for examples/frameworks apps after `glot i18nize` + `glot translate` (mock provider):
// the page at <url> must show the French copy once hydrated, log no console error/warning (hydration
// mismatches are console errors in React 19), and the button must work (the island/app is hydrated).
// usage: node fw_check.mjs <playwright dir> <url>
import { createRequire } from "node:module";

const [pwDir, url] = process.argv.slice(2);
const { chromium } = createRequire(pwDir + "/")("playwright");
const browser = await chromium.launch();
const page = await browser.newPage();
const problems = [];
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") problems.push(`console.${m.type()}: ${m.text()}`);
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
try {
  await page.goto(url, { waitUntil: "networkidle" });
  const count = page.getByText(/You have \d+ open tasks?/);
  await count.waitFor();
  if ((await count.textContent()) !== "⟦fr⟧ You have 1 open task") problems.push(`count: ${await count.textContent()}`);
  if (!(await page.getByText("⟦fr⟧ Ship your projects faster").count())) problems.push("title is not French");
  await page.getByRole("button", { name: "⟦fr⟧ Add a task" }).click();
  await page.getByText("⟦fr⟧ You have 2 open tasks").waitFor({ timeout: 5000 });
} catch (e) {
  problems.push(`check: ${e.message.split("\n")[0]}`);
}
await browser.close();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("ok: French after hydration, no console error, button works");
