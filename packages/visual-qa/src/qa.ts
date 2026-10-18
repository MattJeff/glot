import { chromium, type Page } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

export type Kind = "overflow" | "clamp" | "overlap" | "untranslated" | "tofu" | "rtl";

export interface Issue {
  kind: Kind;
  locale: string;
  route: string;
  viewport: string;
  selector: string;
  text: string;
  key?: string;
  rect?: { x: number; y: number; width: number; height: number };
  crop?: string;
  suggestion?: string;
  command?: string;
}

export interface Options {
  baseUrl: string;
  routes: string[];
  locales: string[];
  sourceLocale: string;
  urlPattern: string;
  viewports: string[];
  out: string;
  suggest: boolean;
}

export interface Report {
  generated_at: string;
  options: Options;
  pages: { locale: string; route: string; viewport: string; url: string; screenshot: string }[];
  issues: Issue[];
}

const VISUAL: Kind[] = ["overflow", "clamp", "overlap"];

/** Source locale gets no prefix (next-intl `as-needed`). */
export function buildUrl(o: Pick<Options, "baseUrl" | "urlPattern" | "sourceLocale">, locale: string, route: string): string {
  const pattern = locale === o.sourceLocale ? o.urlPattern.replace(/\/?\{locale\}/, "") : o.urlPattern;
  let path = pattern.replace("{locale}", locale).replace("{route}", route).replace(/\/{2,}/g, "/");
  if (path.length > 1) path = path.replace(/\/$/, "");
  return o.baseUrl.replace(/\/$/, "") + (path.startsWith("/") ? path : "/" + path);
}

type Found = Omit<Issue, "locale" | "route" | "viewport" | "crop">;

/** Runs inside the page. Must stay self-contained (serialized by Playwright). */
function detect(arg: { sourceTexts: string[] | null; checkRtl: boolean }): { found: Found[]; texts: string[] } {
  const found: Found[] = [];
  const sel = (el: Element): string => {
    const parts: string[] = [];
    for (let e: Element | null = el; e && e !== document.body && parts.length < 5; e = e.parentElement) {
      if (e.id) { parts.unshift("#" + CSS.escape(e.id)); break; }
      const tag = e.tagName.toLowerCase();
      const sibs = e.parentElement ? Array.from(e.parentElement.children).filter((s) => s.tagName === e!.tagName) : [];
      parts.unshift(sibs.length > 1 ? `${tag}:nth-of-type(${sibs.indexOf(e) + 1})` : tag);
    }
    return parts.join(" > ");
  };
  const key = (el: Element) => el.closest("[data-glot-key]")?.getAttribute("data-glot-key") ?? undefined;
  const box = (r: DOMRect) => ({ x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height });

  const texts: string[] = [];
  const seenTofu = new Set<string>();
  const source = arg.sourceTexts ? new Set(arg.sourceTexts) : null;
  const els = Array.from(document.body.querySelectorAll("*")).filter(
    (el) => !["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE"].includes(el.tagName),
  );

  for (const el of els) {
    const own = Array.from(el.childNodes).filter((n) => n.nodeType === 3 && n.textContent!.trim());
    if (!own.length) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    const text = (el as HTMLElement).innerText?.trim() || el.textContent!.trim();
    const cs = getComputedStyle(el);
    for (const n of own) texts.push(n.textContent!.trim());

    const clipped = /hidden|clip/.test(cs.overflowX + cs.overflowY) || cs.textOverflow === "ellipsis";
    const hOver = el.scrollWidth > el.clientWidth + 1;
    // Vertical slack of half a line: tight leading and fallback-font glyphs poke out by a few px without any real clipping.
    const vTol = parseFloat(cs.fontSize) / 2 || 4;
    const vOver = el.scrollHeight > el.clientHeight + vTol;
    const clamp = cs.getPropertyValue("-webkit-line-clamp");
    if (clamp && clamp !== "none" && vOver) {
      found.push({ kind: "clamp", selector: sel(el), text, key: key(el), rect: box(r) });
    } else if (clipped && (hOver || vOver)) {
      found.push({ kind: "overflow", selector: sel(el), text, key: key(el), rect: box(r) });
    } else {
      // Visible overflow: the text itself spills out of its box.
      const range = document.createRange();
      range.selectNodeContents(el);
      const t = range.getBoundingClientRect();
      if (t.right > r.right + 1 || t.left < r.left - 1 || t.bottom > r.bottom + vTol) {
        const u = new DOMRect(Math.min(r.x, t.x), Math.min(r.y, t.y), Math.max(r.right, t.right) - Math.min(r.x, t.x), Math.max(r.bottom, t.bottom) - Math.min(r.y, t.y));
        found.push({ kind: "overflow", selector: sel(el), text, key: key(el), rect: box(u) });
      }
    }

    // ponytail: document.fonts.check only knows declared webfonts; system-font tofu is invisible to it.
    const tk = cs.fontFamily + "|" + text;
    if (!seenTofu.has(tk) && /[^\u0000-ɏ\s\p{P}\p{S}\d]/u.test(text)) {
      seenTofu.add(tk);
      if (!document.fonts.check(`${cs.fontSize} ${cs.fontFamily}`, text)) {
        found.push({ kind: "tofu", selector: sel(el), text, key: key(el) });
      }
    }

    // Code samples and translate="no" content are meant to stay verbatim.
    if (source && !el.closest("pre, code, kbd, samp, [translate=no]")) {
      for (const n of own) {
        const s = n.textContent!.trim();
        if (s.length >= 4 && /\p{L}{2}/u.test(s) && source.has(s)) {
          found.push({ kind: "untranslated", selector: sel(el), text: s, key: key(el) });
        }
      }
    }
  }

  // ponytail: O(n²) over buttons, fine for a page's worth.
  const buttons = Array.from(document.querySelectorAll("button, [role=button], input[type=submit], a.btn, a.button"))
    .map((el) => ({ el, r: el.getBoundingClientRect() }))
    .filter((b) => b.r.width > 0 && b.r.height > 0);
  for (let i = 0; i < buttons.length; i++) {
    for (let j = i + 1; j < buttons.length; j++) {
      const a = buttons[i], b = buttons[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
      const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
      if (w > 1 && h > 1) {
        const u = new DOMRect(Math.min(a.r.x, b.r.x), Math.min(a.r.y, b.r.y), Math.max(a.r.right, b.r.right) - Math.min(a.r.x, b.r.x), Math.max(a.r.bottom, b.r.bottom) - Math.min(a.r.y, b.r.y));
        found.push({ kind: "overlap", selector: `${sel(a.el)} ⨯ ${sel(b.el)}`, text: `${(a.el as HTMLElement).innerText} ⨯ ${(b.el as HTMLElement).innerText}`, key: key(a.el), rect: box(u) });
      }
    }
  }

  if (arg.checkRtl) {
    const dir = document.documentElement.getAttribute("dir") || document.body.getAttribute("dir");
    if (dir !== "rtl") found.push({ kind: "rtl", selector: "html", text: `dir="${dir ?? ""}" (expected "rtl")` });
  }
  return { found, texts };
}

const RTL = new Set(["ar", "he", "fa", "ur"]);

async function grab(page: Page, url: string) {
  await page.goto(url, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
}

async function suggest(issue: Issue, png: Buffer, apiKey: string): Promise<string | undefined> {
  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
      "content-type": "application/json",
      ...(process.env.ANTHROPIC_WORKSPACE_ID ? { "anthropic-workspace-id": process.env.ANTHROPIC_WORKSPACE_ID } : {}),
    },
    body: JSON.stringify({
      model: "claude-sonnet-5-5",
      max_tokens: 200,
      messages: [{
        role: "user",
        content: [
          { type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } },
          {
            type: "text",
            text: `This UI text in locale "${issue.locale}" overflows its container (${Math.round(issue.rect?.width ?? 0)}px wide, viewport ${issue.viewport}).\nText: ${JSON.stringify(issue.text)}\nPropose a shorter ${issue.locale} version with the same meaning that fits. Keep any {placeholders} unchanged. Reply with the text only.`,
          },
        ],
      }],
    }),
  });
  if (!res.ok) { console.warn(`suggest: HTTP ${res.status} ${await res.text()}`); return; }
  const json = (await res.json()) as { content: { type: string; text?: string }[] };
  return json.content.find((c) => c.type === "text")?.text?.trim();
}

export async function run(o: Options): Promise<Report> {
  const shots = join(o.out, "screenshots");
  await mkdir(shots, { recursive: true });
  const browser = await chromium.launch();
  const report: Report = { generated_at: new Date().toISOString(), options: o, pages: [], issues: [] };
  const apiKey = process.env.ANTHROPIC_API_KEY;
  if (o.suggest && !apiKey) console.warn("--suggest: ANTHROPIC_API_KEY not set, skipping suggestions");

  try {
    for (const vp of o.viewports) {
      const [width, height] = vp.split("x").map(Number);
      const page = await browser.newPage({ viewport: { width, height } });
      for (const route of o.routes) {
        // Locale-prefixed visits leave a locale cookie (next-intl NEXT_LOCALE) that would turn the source page into the last locale.
        await page.context().clearCookies();
        await grab(page, buildUrl(o, o.sourceLocale, route));
        const sourceTexts = (await page.evaluate(detect, { sourceTexts: null, checkRtl: false })).texts;

        for (const locale of o.locales) {
          const url = buildUrl(o, locale, route);
          await grab(page, url);
          const { found } = await page.evaluate(detect, { sourceTexts, checkRtl: RTL.has(locale.split("-")[0]) });
          const slug = `${locale}_${route.replace(/[^\w]+/g, "_").replace(/^_|_$/g, "") || "index"}_${vp}`;
          const screenshot = join("screenshots", `${slug}.png`);
          await page.screenshot({ path: join(o.out, screenshot), fullPage: true });
          report.pages.push({ locale, route, viewport: vp, url, screenshot });

          for (const [i, f] of found.entries()) {
            const issue: Issue = { ...f, locale, route, viewport: vp };
            if (VISUAL.includes(f.kind) && f.rect) {
              const pad = 8;
              const clip = { x: Math.max(0, f.rect.x - pad), y: Math.max(0, f.rect.y - pad), width: f.rect.width + 2 * pad, height: f.rect.height + 2 * pad };
              issue.crop = join("screenshots", `${slug}_${i}_${f.kind}.png`);
              const png = await page.screenshot({ path: join(o.out, issue.crop), clip, fullPage: true });
              if (o.suggest && apiKey && f.kind !== "overlap") {
                issue.suggestion = await suggest(issue, png, apiKey);
                if (issue.suggestion && issue.key) issue.command = `glot set ${locale} ${issue.key} ${JSON.stringify(issue.suggestion)}`;
              }
            }
            report.issues.push(issue);
          }
        }
      }
      await page.close();
    }
  } finally {
    await browser.close();
  }

  await writeFile(join(o.out, "report.json"), JSON.stringify(report, null, 2));
  await writeFile(join(o.out, "report.md"), toMarkdown(report));
  return report;
}

const cell = (s: string) => s.replace(/\|/g, "\\|").replace(/\s+/g, " ").slice(0, 120);

export function toMarkdown(r: Report): string {
  const lines = ["## glot visual QA", ""];
  const kinds: Kind[] = ["overflow", "clamp", "overlap", "untranslated", "tofu", "rtl"];
  lines.push(`| Locale | ${kinds.join(" | ")} |`, `|---|${kinds.map(() => "---:").join("|")}|`);
  for (const l of r.options.locales) {
    const mine = r.issues.filter((i) => i.locale === l);
    lines.push(`| \`${l}\` | ${kinds.map((k) => mine.filter((i) => i.kind === k).length || "·").join(" | ")} |`);
  }
  for (const l of r.options.locales) {
    const mine = r.issues.filter((i) => i.locale === l);
    if (!mine.length) continue;
    lines.push("", `<details><summary><b>${l}</b> — ${mine.length} issue(s)</summary>`, "");
    lines.push("| Route | Viewport | Kind | Element | Text | Suggestion |", "|---|---|---|---|---|---|");
    for (const i of mine) {
      const el = i.key ? `\`${i.key}\`` : `\`${cell(i.selector)}\``;
      const sug = i.command ? `\`${cell(i.command)}\`` : i.suggestion ? cell(i.suggestion) : "";
      lines.push(`| ${i.route} | ${i.viewport} | ${i.kind} | ${el} | ${cell(i.text)} | ${sug} |`);
    }
    lines.push("", "</details>");
  }
  if (!r.issues.length) lines.push("", "No issues found.");
  return lines.join("\n") + "\n";
}
