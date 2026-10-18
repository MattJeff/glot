import AxeBuilder from "@axe-core/playwright";
import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { BRIDGE_SNIPPET } from "../lib/preview-bridge";

// Review page (/app/[slug]/feedback): live site in a frame + annotations (docs/design/live-review-contract.md).
// The customer site (http://site.test, preview bridge installed) and the review API are stubbed, so this runs
// against any glot-server; auth + project come from e2e/setup.ts.
const slug = process.env.E2E_SLUG!;
const SITE = "http://site.test";
test.use({ storageState: "e2e/.auth.json" });

const page_ = (title: string) => `<!doctype html><html lang="fr"><head><meta charset="utf-8"><title>Site</title>${BRIDGE_SNIPPET}</head>
<body style="font:16px system-ui;margin:0;padding:40px"><h1 id="title">${title}</h1><p class="subtitle">Bonjour Ada</p>
<a id="cta" href="#" data-glot-key="cta.start">Commencer l'essai gratuit</a></body></html>`;

type Ann = Record<string, unknown> & { id: string; status: string; comments: unknown[] };
async function stub(ctx: BrowserContext) {
  const now = new Date().toISOString();
  const actor = { user_id: "u-other", link_id: null, name: "Grace Hopper" };
  const anns: Ann[] = [
    {
      id: "a1", status: "open", route: "/", locale: "fr", url: `${SITE}/fr`, key: "cta.start", source: "Start free trial {name}",
      translation: { value: "Commencer l'essai gratuit", status: "machine" }, body: "Too long on mobile", body_html: "Too long on mobile",
      suggestion: null, author: actor, assignee: null, feedback_group_id: null, comment_count: 0, attachments: [], reactions: [], approvals: 0,
      created_at: now, updated_at: now, resolved_at: null, comments: [],
      anchor: { key: "cta.start", selector: "#cta", text: "Commencer l'essai gratuit", url: `${SITE}/fr`, locale: "fr", viewport: { w: 1280, h: 800 } },
    },
  ];
  const calls: string[] = [];
  await ctx.route(`${SITE}/**`, (r) => r.fulfill({ contentType: "text/html", body: page_("Livrez votre produit") }));
  await ctx.route(`**/api/projects/${slug}/preview`, (r) =>
    r.request().method() === "GET" ? r.fulfill({ json: { base_url: SITE, url_pattern: "/{locale}{route}", routes: ["/"] } }) : r.fallback(),
  );
  await ctx.route(`**/api/projects/${slug}/review/**`, async (r) => {
    const req = r.request();
    const path = new URL(req.url()).pathname.replace(`/api/projects/${slug}/review`, "");
    const m = req.method();
    calls.push(`${m} ${path}`);
    const body = req.postData() ? JSON.parse(req.postData()!) : {};
    if (path === "/events") return r.fulfill({ status: 200, contentType: "text/event-stream", body: ": ok\n\n" });
    if (path === "/members") return r.fulfill({ json: [{ user_id: "u-other", name: "Grace Hopper" }] });
    if (path === "/session") return r.fulfill({ json: { token: "glr_test", expires_at: now } });
    if (path === "/annotations" && m === "GET") return r.fulfill({ json: { items: anns, total: anns.length } });
    if (path === "/annotations" && m === "POST") {
      const a: Ann = { ...anns[0], id: `a${anns.length + 1}`, body: body.body, body_html: body.body, anchor: body.anchor, key: null, author: { ...actor, name: "A11y Bot" }, created_at: new Date().toISOString() };
      anns.push(a);
      return r.fulfill({ status: 201, json: a });
    }
    const one = path.match(/^\/annotations\/([^/]+)$/);
    if (one) {
      const a = anns.find((x) => x.id === one[1])!;
      if (m === "PATCH") Object.assign(a, body, body.status === "resolved" ? { resolved_at: new Date().toISOString() } : {});
      return r.fulfill({ json: a });
    }
    if (path === "/text/cta.start" && m === "GET")
      return r.fulfill({
        json: {
          key: "cta.start", source: "Start free trial {name}", source_locale: "en", description: null, ui_role: null,
          values: Object.fromEntries(["en", "fr", "de", "ar"].map((l) => [l, { value: l === "fr" ? "Commencer l'essai gratuit {name}" : "", status: l === "fr" ? "machine" : "missing", score: null, issues: [], origin: "machine", model: null, updated_at: now }])),
        },
      });
    if (path === "/text/cta.start" && m === "PUT")
      return r.fulfill({ json: { results: Object.fromEntries(Object.keys(body.values).map((l) => [l, { issues: [], translation: null }])) } });
    return r.fulfill({ status: 404, json: { error: { code: "not_found", message: "stub" } } });
  });
  return calls;
}

async function open(page: Page) {
  await page.goto(`/en/app/${slug}/feedback`);
  await page.getByRole("heading", { name: "Review", level: 1 }).waitFor();
}

async function axe(page: Page) {
  const { violations } = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .exclude("nextjs-portal")
    .exclude("iframe") // the customer's site is not ours to audit
    .analyze();
  const blocking = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  return blocking.map((v) => `${v.id} (${v.impact}): ${v.help}\n  ${v.nodes.slice(0, 3).map((n) => n.target.join(" ")).join("\n  ")}`);
}

for (const scheme of ["light", "dark"] as const) {
  test(`review: axe ${scheme} (list, detail, editor, share)`, async ({ page, context }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await stub(context);
    await open(page);
    await expect(page.getByRole("button", { name: /^Annotation 1: Too long on mobile/ }).first()).toBeVisible();
    expect(await axe(page)).toEqual([]);
    await page.getByRole("list", { name: "Annotations" }).getByRole("button").first().click();
    await expect(page.getByRole("heading", { name: "Annotation 1" })).toBeVisible();
    expect(await axe(page)).toEqual([]);
    await page.getByRole("button", { name: "Edit text" }).last().click();
    await expect(page.getByRole("textbox", { name: /French/ })).toBeVisible();
    expect(await axe(page)).toEqual([]);
    await page.getByRole("button", { name: "Back" }).click();
    await page.getByRole("button", { name: "Share" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    expect(await axe(page)).toEqual([]);
  });
}

test("review: pin on the live site, annotate with C, navigate with J/K, resolve with R", async ({ page, context }) => {
  const calls = await stub(context);
  await open(page);
  // The bridge reports the anchored element's position: the pin sits over the iframe.
  const pin = page.getByRole("button", { name: /^Annotation 1: Too long on mobile/ }).first();
  await expect(pin).toBeVisible();
  await page.keyboard.press("c");
  await expect(page.getByText("Click the text you want")).toBeVisible();
  await page.frameLocator("iframe").getByRole("heading", { name: "Livrez votre produit" }).click();
  const body = page.getByLabel("Comment", { exact: true });
  await expect(body).toBeFocused();
  await body.fill("Use “Lancez” instead");
  await page.getByRole("button", { name: "Add annotation" }).click();
  await expect(page.getByRole("heading", { name: "Annotation 2" })).toBeVisible();
  expect(calls).toContain("POST /annotations");
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("k");
  await expect(page.getByRole("heading", { name: "Annotation 1" })).toBeVisible();
  await page.keyboard.press("r");
  await expect.poll(() => calls.filter((c) => c === "PATCH /annotations/a1").length).toBe(1);
});

test("review: manual edit protects placeholders and previews live", async ({ page, context }) => {
  await stub(context);
  await open(page);
  await page.getByRole("list", { name: "Annotations" }).getByRole("button").first().click();
  await page.keyboard.press("e");
  const fr = page.getByRole("textbox", { name: /French/ });
  await fr.fill("Essai gratuit");
  await expect(page.getByText("Placeholder {name} is missing.")).toBeVisible();
  await expect(page.getByRole("button", { name: /Save/ })).toBeDisabled();
  await page.getByRole("region", { name: /French/ }).getByRole("button", { name: "Insert {name}" }).click();
  await expect(fr).toHaveValue(/\{name\}/);
  await expect(page.getByText("Looks good")).toBeVisible();
  await expect(page.frameLocator("iframe").locator("#cta")).toHaveText(/Essai gratuit/);
});

test("review: phone layout, frame full width + panel in a drawer", async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await stub(context);
  await open(page);
  const scroll = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(scroll).toBeLessThanOrEqual(0);
  await page.getByRole("button", { name: /open annotation/ }).click();
  await expect(page.getByRole("dialog", { name: "Annotations" })).toBeVisible();
  expect(await axe(page)).toEqual([]);
});
