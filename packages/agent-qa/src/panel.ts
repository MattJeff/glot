// `glot labs panel` capture: per route × locale, an above-the-fold screenshot, the page text and its
// calls to action (with their catalog key). The synthetic personas (LLM side) live in glot (Rust).
import type { Browser } from "playwright";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { parse } from "yaml";
import { buildUrl, launch } from "../../visual-qa/src/qa.ts";
import { buildIndex, flatten, match } from "../../visual-qa/src/keys.ts";

export interface Cta { id: number; text: string; href?: string; key?: string }
export interface PageCapture { route: string; locale: string; url: string; screenshot: string; title: string; headline: string; text: string; ctas: Cta[] }
/** glot.panel.yaml, passed through to glot (which merges personas with its defaults). */
export interface PanelConfig {
  product?: string;
  routes?: { route: string; value_proposition?: string }[];
  personas?: Record<string, unknown>;
}
export interface CaptureOptions {
  baseUrl: string;
  routes: string[];
  locales: string[];
  sourceLocale: string;
  urlPattern: string;
  localeCookie?: string;
  catalogs?: string;
  viewport: string;
  out: string;
}

const MAX_TEXT = 4000;
const MAX_CTAS = 12;

export async function loadConfig(path: string | undefined): Promise<PanelConfig> {
  if (!path) return {};
  try {
    const doc = parse(await readFile(path, "utf8")) ?? {};
    if (typeof doc !== "object" || Array.isArray(doc)) throw new Error(`${path}: expected a mapping (product, routes, personas)`);
    doc.routes = (doc.routes ?? []).map((r: unknown) => (typeof r === "string" ? { route: r } : r));
    for (const r of doc.routes) if (!r?.route?.startsWith?.("/")) throw new Error(`${path}: each route needs a path starting with "/"`);
    return doc;
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return {};
    throw e;
  }
}

/** Runs in the page: what a visitor reads, and what they can click. Self-contained (serialized). */
function read(max: { text: number; ctas: number }) {
  const visible = (el: Element) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
  };
  const label = (el: Element) =>
    ((el as HTMLElement).innerText || (el as HTMLInputElement).value || el.getAttribute("aria-label") || "").replace(/\s+/g, " ").trim();
  const seen = new Set<string>();
  const ctas: { text: string; href?: string }[] = [];
  const sel = "a[href], button, [role=button], input[type=submit]";
  // The page's own actions before the navigation chrome.
  const main = document.querySelector("main, [role=main]");
  for (const el of new Set([...Array.from(main?.querySelectorAll(sel) ?? []), ...Array.from(document.querySelectorAll(sel))])) {
    const text = label(el);
    const href = el.getAttribute("href") ?? undefined;
    const id = `${text}|${href ?? ""}`;
    if (!text || seen.has(id) || !visible(el)) continue;
    seen.add(id);
    ctas.push({ text, href });
    if (ctas.length >= max.ctas) break;
  }
  const h1 = document.querySelector("h1");
  return {
    title: document.title,
    headline: [h1 ? label(h1) : "", document.querySelector('meta[name="description"]')?.getAttribute("content") ?? ""].filter(Boolean).join(" — "),
    text: document.body.innerText.replace(/\n{3,}/g, "\n\n").trim().slice(0, max.text),
    ctas,
  };
}

async function loadCatalogs(patterns: string | undefined, locales: string[]) {
  const out: Record<string, Record<string, string>> = {};
  for (const l of locales) {
    out[l] = {};
    for (const p of (patterns ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
      try { flatten(JSON.parse(await readFile(p.replaceAll("{locale}", l), "utf8")), "", out[l]); }
      catch (e) { if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e; }
    }
  }
  return out;
}

export async function capture(o: CaptureOptions, browser?: Browser): Promise<PageCapture[]> {
  const cats = await loadCatalogs(o.catalogs, o.locales);
  const shots = join(o.out, "screenshots");
  await mkdir(shots, { recursive: true });
  const [width, height] = o.viewport.split("x").map(Number);
  const own = !browser;
  const b = browser ?? (await launch());
  const cookies = (o.localeCookie ?? "").split(",").map((s) => s.trim()).filter(Boolean);
  const pages: PageCapture[] = [];
  try {
    for (const route of o.routes) {
      for (const locale of o.locales) {
        const ctx = await b.newContext({ viewport: { width, height }, locale });
        if (cookies.length) await ctx.addCookies(cookies.map((name) => ({ name, value: locale, url: o.baseUrl })));
        const page = await ctx.newPage();
        const url = buildUrl(o, locale, route);
        await page.goto(url, { waitUntil: "load" });
        await page.waitForLoadState("networkidle", { timeout: 5000 }).catch(() => {});
        const r = await page.evaluate(read, { text: MAX_TEXT, ctas: MAX_CTAS });
        const slug = `${locale}_${route.replace(/[^a-z0-9]+/gi, "_").replace(/^_|_$/g, "") || "home"}`;
        const screenshot = join("screenshots", `${slug}.png`);
        // Above the fold only: the 5-second test is about what a visitor sees first.
        await page.screenshot({ path: join(o.out, screenshot) });
        const index = buildIndex(cats[locale] ?? {});
        // Icons around a label ("◧ Dashboard", "See plans →") are not part of the message.
        const bare = (t: string) => t.replace(/^[^\p{L}\p{N}⟦]+|[^\p{L}\p{N}.!?)]+$/gu, "");
        const ctas = r.ctas.map((c, id) => ({ id, ...c, key: match(index, c.text) ?? match(index, bare(c.text)) }));
        pages.push({ route, locale, url, screenshot, title: r.title, headline: r.headline, text: r.text, ctas });
        await ctx.close();
      }
    }
  } finally {
    if (own) await b.close();
  }
  return pages;
}

/** `glot-agent-qa panel …`: writes `<out>/capture.json` = {config, pages}. */
export async function main(argv: string[]) {
  const { values: v } = parseArgs({
    args: argv,
    options: {
      config: { type: "string", default: "glot.panel.yaml" },
      routes: { type: "string" },
      "base-url": { type: "string", default: "http://localhost:3000" },
      locales: { type: "string", default: "en,fr" },
      "source-locale": { type: "string", default: "en" },
      "url-pattern": { type: "string", default: "/{locale}{route}" },
      "locale-cookie": { type: "string", default: "NEXT_LOCALE,glot-locale" },
      catalogs: { type: "string" },
      viewport: { type: "string", default: "1280x800" },
      out: { type: "string", default: ".glot/panel" },
    },
  });
  const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
  const config = await loadConfig(v.config);
  const routes = v.routes ? list(v.routes) : config.routes?.length ? config.routes.map((r) => r.route) : ["/"];
  const pages = await capture({
    baseUrl: v["base-url"]!,
    routes,
    locales: list(v.locales!),
    sourceLocale: v["source-locale"]!,
    urlPattern: v["url-pattern"]!,
    localeCookie: v["locale-cookie"],
    catalogs: v.catalogs,
    viewport: v.viewport!,
    out: v.out!,
  });
  await writeFile(join(v.out!, "capture.json"), JSON.stringify({ config, pages }, null, 2));
  console.log(`panel: captured ${pages.length} page(s) — ${join(v.out!, "capture.json")}`);
}
