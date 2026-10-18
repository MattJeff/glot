import { mkdirSync, writeFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";

// Responsive gate: every route at phone → desktop widths, light/dark and Arabic (RTL).
// Blocking: page-level horizontal scroll, elements poking out of the viewport, touch targets
// < 24 px (WCAG 2.5.8), controls hidden under sticky/fixed layers, dialogs larger than the screen.
// Warnings (reported, not failing): targets < 44 px on phones, ellipsis without a tooltip.
// RESPONSIVE_FULL=1 runs all 8 widths × light/dark/ar; SHOTS=<dir> saves a screenshot per problem.
const slug = process.env.E2E_SLUG!;
const client = process.env.E2E_OAUTH_CLIENT;
const FULL = !!process.env.RESPONSIVE_FULL;
const SHOTS = process.env.SHOTS;
const WIDTHS = FULL ? [320, 375, 414, 768, 1024, 1280, 1440, 1920] : [320, 375, 768, 1024, 1440];
const VARIANTS = FULL ? (["light", "dark", "ar"] as const) : (["light", "ar"] as const);

test.use({ storageState: "e2e/.auth.json" });

const ROUTES = [
  "/",
  "/i18n",
  "/pricing",
  "/docs",
  "/docs/getting-started",
  "/docs/cli",
  "/login",
  "/signup",
  "/forgot-password",
  "/reset-password?token=x",
  "/verify-email?token=x",
  "/app",
  `/app/${slug}`,
  `/app/${slug}/onboarding`,
  `/app/${slug}/keys`,
  `/app/${slug}/preview`,
  `/app/${slug}/visual`,
  `/app/${slug}/glossary`,
  `/app/${slug}/jobs`,
  `/app/${slug}/github`,
  `/app/${slug}/settings`,
  `/app/${slug}/insights`,
  `/app/${slug}/experiments`,
  `/app/${slug}/feedback`,
  "/app/settings/account",
  "/app/settings/tokens",
  "/app/settings/billing",
  ...(client
    ? [`/oauth/authorize?response_type=code&client_id=${client}&redirect_uri=${encodeURIComponent("http://127.0.0.1:9/cb")}&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cQ&code_challenge_method=S256&scope=read`]
    : []),
];

type Problem = { kind: string; blocking: boolean; el: string; detail: string };

// Runs in the page (serialised by page.evaluate: keep it self-contained): returns layout problems for the current viewport.
function audit(): Problem[] {
  const out: Problem[] = [];
  const W = window.innerWidth;
  const H = window.innerHeight;
  const mobile = W < 768;
  const name = (el: Element) => {
    const id = el.id ? `#${el.id}` : "";
    const cls = typeof el.className === "string" ? el.className.split(/\s+/).slice(0, 4).join(".") : "";
    const txt = (el.getAttribute("aria-label") || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, 40);
    return `${el.tagName.toLowerCase()}${id}${cls ? "." + cls : ""} "${txt}"`;
  };
  const visible = (el: Element) => {
    const s = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    return s.visibility !== "hidden" && s.display !== "none" && +s.opacity > 0.05 && r.width > 0 && r.height > 0 && !el.closest("[aria-hidden=true],[inert],.sr-only,nextjs-portal");
  };
  // Is the element inside something that clips or scrolls horizontally?
  const clipped = (el: Element) => {
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const o = getComputedStyle(p).overflowX;
      if (o !== "visible") return true;
    }
    return false;
  };

  const doc = document.documentElement;
  if (doc.scrollWidth > W + 1) out.push({ kind: "page-hscroll", blocking: true, el: "html", detail: `scrollWidth ${doc.scrollWidth} > ${W}` });

  const all = [...document.body.querySelectorAll("*")];
  const reported = new Set<Element>();
  for (const el of all) {
    if (!visible(el) || getComputedStyle(el).position === "fixed") continue;
    const r = el.getBoundingClientRect();
    if ((r.right > W + 1 || r.left < -1) && !clipped(el) && ![...reported].some((p) => p.contains(el))) {
      reported.add(el);
      out.push({ kind: "overflow", blocking: true, el: name(el), detail: `x ${Math.round(r.left)}→${Math.round(r.right)} of ${W}` });
    }
  }

  const TARGETS = "a[href],button,input:not([type=hidden]),select,textarea,[role=button],[role=tab],[role=switch],[role=checkbox],[role=menuitem],[role=combobox],[role=option],summary";
  const targets = [...document.querySelectorAll(TARGETS)].filter((el) => visible(el) && !el.closest("[data-slot=command-list]") && !el.matches("[disabled]"));
  const rects = targets.map((el) => el.getBoundingClientRect());
  targets.forEach((el, i) => {
    const r = rects[i];
    // Inline links inside running text are exempt (2.5.8 "inline").
    const inline = el.tagName === "A" && getComputedStyle(el).display === "inline" && !!el.parentElement?.closest("p,li,td,label,small,span");
    if (inline) return;
    if (r.width < 24 || r.height < 24) {
      // Spacing exception: a 24 px circle centred on the target must not hit another target.
      const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
      const crowded = rects.some((o, j) => {
        if (j === i || targets[j].contains(el) || el.contains(targets[j])) return false;
        const dx = Math.max(o.left - cx, 0, cx - o.right), dy = Math.max(o.top - cy, 0, cy - o.bottom);
        return Math.hypot(dx, dy) < 12;
      });
      if (crowded) out.push({ kind: "target<24", blocking: true, el: name(el), detail: `${Math.round(r.width)}×${Math.round(r.height)}` });
    } else if (mobile && (r.width < 44 || r.height < 44) && el.closest("main,header,nav,[role=dialog]")) {
      out.push({ kind: "target<44", blocking: false, el: name(el), detail: `${Math.round(r.width)}×${Math.round(r.height)}` });
    }
    // Focused control hidden under a sticky/fixed layer (WCAG 2.4.11).
    if (el === document.activeElement && r.top >= 0 && r.bottom <= H && r.left >= 0 && r.right <= W && r.width >= 4 && r.height >= 4) {
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (hit && !el.contains(hit) && !hit.contains(el) && !(el as HTMLInputElement).labels?.[0]?.contains(hit)) {
        const layer = hit.closest("header,[data-sticky],nav,footer,[class*=sticky],[class*=fixed]");
        if (layer && !layer.contains(el)) out.push({ kind: "obscured", blocking: true, el: name(el), detail: `under ${name(layer)}` });
      }
    }
  });

  // Ellipsis without a way to read the full text.
  for (const el of all) {
    if (!visible(el) || !el.childNodes.length) continue;
    const s = getComputedStyle(el);
    const trunc = (s.textOverflow === "ellipsis" && el.scrollWidth > el.clientWidth + 1) || (s.webkitLineClamp !== "none" && el.scrollHeight > el.clientHeight + 1);
    if (trunc && !el.closest("[title],[aria-label],[data-slot=tooltip-trigger],[aria-describedby]")) {
      out.push({ kind: "truncated", blocking: false, el: name(el), detail: `${el.scrollWidth}>${el.clientWidth}` });
    }
  }

  for (const d of document.querySelectorAll("[role=dialog],[role=alertdialog]")) {
    if (!visible(d)) continue;
    const r = d.getBoundingClientRect();
    if (r.left < -1 || r.right > W + 1 || r.height > H + 1) out.push({ kind: "dialog-overflow", blocking: true, el: name(d), detail: `${Math.round(r.width)}×${Math.round(r.height)} in ${W}×${H}` });
  }
  return out;
}

async function check(page: Page, label: string) {
  const problems = await page.evaluate(audit);
  if (SHOTS && problems.some((p) => p.blocking)) {
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: `${SHOTS}/${label.replace(/[^\w-]+/g, "_")}.png`, fullPage: true });
  }
  return problems.map((p) => ({ ...p, at: label }));
}

for (const variant of VARIANTS) {
  for (const route of ROUTES) {
    test(`responsive ${variant} ${route.split("?")[0]}`, async ({ page }) => {
      test.setTimeout(120_000);
      const locale = variant === "ar" ? "ar" : "en";
      await page.emulateMedia({ colorScheme: variant === "dark" ? "dark" : "light", reducedMotion: "reduce" });
      await page.setViewportSize({ width: WIDTHS[0], height: 800 });
      const res = await page.goto(`/${locale}${route === "/" ? "" : route}`);
      expect(res?.status(), "page must render").toBeLessThan(500);
      await page.locator("h1").first().waitFor({ timeout: 30_000 }).catch(() => {});
      await page.waitForLoadState("networkidle");
      const found = [];
      for (const w of WIDTHS) {
        await page.setViewportSize({ width: w, height: w < 768 ? 800 : 900 });
        await page.waitForTimeout(150);
        const label = `${variant}-${w}-${route.split("?")[0]}`;
        found.push(...(await check(page, label)));
        if (w === 375 && route.startsWith("/app")) {
          // Phone: the command palette must open from a tap and fit the screen.
          const search = page.getByRole("button", { name: /search|بحث/i }).first();
          if (await search.isVisible()) {
            await search.click();
            await page.getByRole("dialog").waitFor();
            found.push(...(await check(page, `${label}-palette`)));
            await page.keyboard.press("Escape");
          }
          // Virtual keyboard: shrink the viewport with an input focused, it must stay unobscured.
          const input = page.locator("main input:visible, main textarea:visible").first();
          if (await input.count()) {
            await input.focus();
            await page.setViewportSize({ width: w, height: 420 });
            await input.evaluate((el) => el.scrollIntoView({ block: "center" }));
            found.push(...(await check(page, `${label}-keyboard`)).filter((p) => p.kind === "obscured"));
            await page.setViewportSize({ width: w, height: 800 });
          }
        }
      }
      const dir = "e2e/.results";
      mkdirSync(dir, { recursive: true });
      writeFileSync(`${dir}/responsive-${variant}-${route.split("?")[0].replace(/\W+/g, "_")}.json`, JSON.stringify(found, null, 1));
      const blocking = found.filter((p) => p.blocking).map((p) => `${p.at} ${p.kind} ${p.el} ${p.detail}`);
      expect([...new Set(blocking)], blocking.join("\n")).toEqual([]);
    });
  }
}
