import { chromium, type Page } from "playwright";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fontFileName, toSfnt } from "./fonts.ts";
import { join } from "node:path";
import { loadIndexes, match, type Index } from "./keys.ts";

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
  /** Crop of the surrounding component (section, form, card…), for multimodal fixes. */
  context_crop?: string;
  /** Overflow geometry: box size vs size the text needs, plus the computed font. */
  fit?: { width: number; height: number; need_width: number; need_height: number; font: string };
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
  /** Catalog paths with `{locale}` (comma separated): maps rendered text back to keys. */
  catalogs?: string;
  /** Crop every element mapped to a key ("*") or to these keys, into `report.keys`. */
  map?: string;
  /** Record, per key rendered in the source locale, the box it may use and its font (`report.layout`), and save webfonts to `fontsDir`. */
  measure?: boolean;
  fontsDir?: string;
  /**
   * Harness mode (`glot coverage --render`): JSON file `{ "<locale>": "<html>" }`. Each fragment replaces the
   * body of the app page at `baseUrl` + first route (source locale, so the app's CSS and webfonts apply) and is
   * checked like a page. Elements carry `data-glot-key` (`key#branch`): issues come back keyed.
   */
  harness?: string;
}

/** Pixel budget of a key (`glot measure` → `.glot/layout.json`). */
export interface Fit {
  max_width_px: number;
  /** null: wraps freely (only each word must fit). */
  max_lines: number | null;
  font: { family: string; postscript?: string; size_px: number; weight: number; letter_spacing_px: number; text_transform?: string };
  /** Source text as rendered and its width: calibrates glot's browser-free measure. */
  text?: string;
  text_px?: number;
  /** Every place the key rendered (route × viewport × element); the budget above is the tightest of them. */
  occurrences?: Occurrence[];
}

export interface Occurrence {
  route: string;
  viewport: string;
  selector: string;
  max_width_px: number;
  max_lines: number | null;
  /** How the element was mapped to the key: dev marker (`GLOT_DEV_KEYS=1`, exact) or rendered text (ambiguous for duplicates). */
  by: "marker" | "text";
}

export interface Mapped {
  locale: string;
  route: string;
  viewport: string;
  key: string;
  text: string;
  crop: string;
}

export interface Report {
  generated_at: string;
  options: Options;
  pages: { locale: string; route: string; viewport: string; url: string; screenshot: string }[];
  issues: Issue[];
  keys?: Mapped[];
  layout?: Record<string, Fit>;
}

const VISUAL: Kind[] = ["overflow", "clamp", "overlap"];

/** Source locale gets no prefix (next-intl `as-needed`). */
/** Chromium, or an error that says how to get it (a fresh `npx` install has no browser yet). */
export async function launch() {
  return chromium.launch().catch((e: Error) => {
    if (/Executable doesn't exist/.test(e.message)) throw new Error(`Playwright's Chromium is missing: run \`npx playwright install chromium\`\n${e.message.split("\n")[0]}`);
    throw e;
  });
}

export function buildUrl(o: Pick<Options, "baseUrl" | "urlPattern" | "sourceLocale">, locale: string, route: string): string {
  // The source drops its `/{locale}` segment or its `?lang={locale}` param.
  const pattern = locale === o.sourceLocale
    ? o.urlPattern.replace(/[?&][^?&=#]+=\{locale\}/, "").replace(/^([^?]*)&/, "$1?").replace(/\/?\{locale\}/, "")
    : o.urlPattern;
  let path = pattern.replace("{locale}", locale).replace("{route}", route).replace(/\/{2,}/g, "/");
  if (path.length > 1) path = path.replace(/\/$/, "");
  return o.baseUrl.replace(/\/$/, "") + (path.startsWith("/") ? path : "/" + path);
}

type Rect = { x: number; y: number; width: number; height: number };
type Found = Omit<Issue, "locale" | "route" | "viewport" | "crop" | "context_crop"> & { ctx?: Rect };
type El = { text: string; key?: string; rect: Rect };

/** Runs inside the page. Must stay self-contained (serialized by Playwright). */
function detect(arg: { sourceTexts: string[] | null; checkRtl: boolean }): { found: Found[]; texts: string[]; els: El[] } {
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
  // Surrounding component: nearest semantic container, else the grandparent.
  const ctx = (el: Element) =>
    box((el.parentElement?.closest("section, form, nav, header, footer, aside, article, dialog, [role=dialog], li, tr, fieldset")
      ?? el.parentElement?.parentElement ?? el).getBoundingClientRect());
  const shown: El[] = [];
  // Ellipsis / line-clamp on the box that clips: truncation by design, reported as "clamp", not "overflow".
  const truncates = (s: CSSStyleDeclaration) => s.textOverflow === "ellipsis" || !["", "none"].includes(s.getPropertyValue("-webkit-line-clamp"));
  const clipperTruncates = (el: Element | null) => {
    for (; el; el = el.parentElement) {
      const s = getComputedStyle(el);
      if (/hidden|clip/.test(s.overflowX + s.overflowY)) return truncates(s);
    }
    return false;
  };

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
    // Visually hidden (sr-only): a 1px clipped box by design, never a layout bug, nothing to show.
    const hidden = r.width <= 2 || r.height <= 2;
    if (!hidden) shown.push({ text, key: key(el), rect: box(r) });
    const geo = (w: number, h: number) => ({ width: Math.round(el.clientWidth || r.width), height: Math.round(el.clientHeight || r.height), need_width: Math.round(w), need_height: Math.round(h), font: cs.font || `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}` });

    const clipped = /hidden|clip/.test(cs.overflowX + cs.overflowY) || cs.textOverflow === "ellipsis";
    const hOver = el.scrollWidth > el.clientWidth + 1;
    // Vertical slack of half a line: tight leading and fallback-font glyphs poke out by a few px without any real clipping.
    const vTol = parseFloat(cs.fontSize) / 2 || 4;
    const vOver = el.scrollHeight > el.clientHeight + vTol;
    const clamp = cs.getPropertyValue("-webkit-line-clamp");
    if (hidden) {
      // skip
    } else if (clamp && clamp !== "none" && vOver) {
      found.push({ kind: "clamp", selector: sel(el), text, key: key(el), rect: box(r), ctx: ctx(el), fit: geo(el.scrollWidth, el.scrollHeight) });
    } else if (clipped && (hOver || vOver)) {
      // Ellipsis only covers the inline axis: text cut vertically is still overflow.
      const byDesign = /hidden|clip/.test(cs.overflowX) && cs.textOverflow === "ellipsis" && !vOver;
      found.push({ kind: byDesign ? "clamp" : "overflow", selector: sel(el), text, key: key(el), rect: box(r), ctx: ctx(el), fit: geo(el.scrollWidth, el.scrollHeight) });
    } else {
      // Visible overflow: the text itself spills out of its box.
      const range = document.createRange();
      range.selectNodeContents(el);
      const t = range.getBoundingClientRect();
      if (t.right > r.right + 1 || t.left < r.left - 1 || t.bottom > r.bottom + vTol) {
        const u = new DOMRect(Math.min(r.x, t.x), Math.min(r.y, t.y), Math.max(r.right, t.right) - Math.min(r.x, t.x), Math.max(r.bottom, t.bottom) - Math.min(r.y, t.y));
        found.push({ kind: clipperTruncates(el.parentElement) ? "clamp" : "overflow", selector: sel(el), text, key: key(el), rect: box(u), ctx: ctx(el), fit: geo(Math.max(t.right, r.right) - Math.min(t.left, r.left), Math.max(t.bottom, r.bottom) - Math.min(t.top, r.top)) });
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
  return { found, texts, els: shown };
}


/**
 * Runs inside the page. Dev key markers (`@glot/runtime/dev`, `GLOT_DEV_KEYS=1`) become `data-glot-key` on the
 * element of the marked text, and are removed so the page renders exactly like production. Self-contained
 * copy of `decodeKeys` (keys.ts).
 */
function unmark(): number {
  const re = /\u2063([\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]+)\u2063/gu;
  const decode = (bytes: string) =>
    new TextDecoder().decode(new Uint8Array(Array.from(bytes, (c) => { const p = c.codePointAt(0)!; return p < 0xe0100 ? p - 0xfe00 : p - 0xe0100 + 16; })));
  let n = 0;
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let t = w.nextNode() as Text | null; t; t = w.nextNode() as Text | null) {
    if (!t.data.includes("\u2063")) continue;
    let first: string | undefined;
    t.data = t.data.replace(re, (_, b: string) => { first ??= decode(b); return ""; });
    const el = t.parentElement;
    if (first !== undefined && el && !el.hasAttribute("data-glot-key")) { el.setAttribute("data-glot-key", first); n++; }
  }
  for (const el of Array.from(document.querySelectorAll("[title], [placeholder], [aria-label], [alt], [value]"))) {
    for (const a of ["title", "placeholder", "aria-label", "alt", "value"]) {
      const v = el.getAttribute(a);
      if (v?.includes("\u2063")) el.setAttribute(a, v.replace(re, ""));
    }
  }
  if (document.title.includes("\u2063")) document.title = document.title.replace(re, "");
  return n;
}

type Probe = Omit<Fit, "font"> & { text: string; ownText: string; font: Fit["font"] & { css: string }; key?: string; selector: string };

/**
 * Runs inside the page. For each element with its own text: the room its text has before it overflows.
 * Probe: lengthen the text 13x and watch the box. Not growing => fixed width; growing then
 * overflowing => it hit a limit (max-width, flex); growing to fit => unconstrained (skipped).
 * Marks measured elements with `data-glot-m` (document order) so CDP can read their real fonts.
 */
function probe(): Probe[] {
  const out: Probe[] = [];
  const px = (v: string) => parseFloat(v) || 0;
  // Same as `detect`'s selector (functions run in the page cannot share helpers).
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
  for (const e of Array.from(document.querySelectorAll("[data-glot-m]"))) e.removeAttribute("data-glot-m");
  const els = Array.from(document.body.querySelectorAll("*")).filter(
    (el) => !["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE"].includes(el.tagName),
  );
  for (const el of els) {
    const own = Array.from(el.childNodes).filter((n) => n.nodeType === 3 && n.textContent!.trim()) as Text[];
    if (!own.length) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 2 || r.height <= 2) continue;
    const cs = getComputedStyle(el);
    // Inline text lives in its nearest block-level box.
    let box = el as HTMLElement;
    while (getComputedStyle(box).display === "inline" && box.parentElement) box = box.parentElement;
    const bs = getComputedStyle(box);
    const padX = px(bs.paddingLeft) + px(bs.paddingRight);
    const padY = px(bs.paddingTop) + px(bs.paddingBottom);
    const rangeOf = (n: Node, whole = false) => {
      const g = document.createRange();
      if (whole) g.selectNodeContents(n); else g.selectNode(n);
      return g;
    };
    const ownW = () => own.reduce((w, n) => w + rangeOf(n).getBoundingClientRect().width, 0);
    const lines0 = own.reduce((m, n) => Math.max(m, new Set(Array.from(rangeOf(n).getClientRects()).map((q) => Math.round(q.top))).size), 0);
    const textW = ownW();
    // Icons and other children share the line: their width is not available to the text.
    const other = lines0 <= 1 ? Math.max(0, rangeOf(box, true).getBoundingClientRect().width - textW) : 0;
    const b0 = box.getBoundingClientRect();
    const cw0 = box.clientWidth - padX, ch0 = box.clientHeight - padY;
    const orig = own[0].data;
    own[0].data = orig + (" " + orig.trim()).repeat(12);
    const b1 = box.getBoundingClientRect();
    const cw1 = box.clientWidth - padX;
    const t1 = rangeOf(box, true).getBoundingClientRect();
    const overflows = box.scrollWidth > box.clientWidth + 1 || t1.right > b1.right + 1 || t1.left < b1.left - 1
      || box.scrollHeight > box.clientHeight + 1 || t1.bottom > b1.bottom + 1;
    own[0].data = orig;
    const grewW = b1.width - b0.width > 0.5, grewH = b1.height - b0.height > 0.5;
    const clamp = parseInt(bs.getPropertyValue("-webkit-line-clamp"));
    const lh = cs.lineHeight === "normal" ? px(cs.fontSize) * 1.2 : px(cs.lineHeight);
    let lines: number | null;
    if (clamp > 0) lines = clamp;
    else if (/nowrap|pre$/.test(cs.whiteSpace + " " + bs.whiteSpace) || /nowrap/.test(cs.textWrap + bs.textWrap)) lines = 1;
    else if (!grewH && overflows) lines = Math.max(1, Math.floor((ch0 + 1) / lh));
    else lines = null;
    let width: number;
    if (!grewW) width = cw0 - other;
    else if (overflows) width = cw1 - other;
    else continue; // the box grows with its text: nothing to overflow
    el.setAttribute("data-glot-m", String(out.length));
    out.push({
      max_width_px: Math.floor(width * 10) / 10,
      max_lines: lines,
      font: {
        family: cs.fontFamily, css: cs.fontFamily, size_px: px(cs.fontSize), weight: parseInt(cs.fontWeight) || 400,
        letter_spacing_px: cs.letterSpacing === "normal" ? 0 : px(cs.letterSpacing),
        ...(cs.textTransform !== "none" ? { text_transform: cs.textTransform } : {}),
      },
      text: (el as HTMLElement).innerText?.trim() || el.textContent!.trim(),
      ownText: own.map((n) => n.data).join(" ").replace(/\s+/g, " ").trim(),
      text_px: lines0 <= 1 ? Math.round(textW * 100) / 100 : undefined,
      key: el.getAttribute("data-glot-key") ?? undefined,
      selector: sel(el),
    });
  }
  return out;
}

/** Real font of each `data-glot-m` element (the one rendering most glyphs), via CDP. */
async function platformFonts(page: Page): Promise<{ familyName: string; postScriptName?: string }[]> {
  const cdp = await page.context().newCDPSession(page);
  try {
    await cdp.send("DOM.enable");
    await cdp.send("CSS.enable");
    const { root } = await cdp.send("DOM.getDocument", { depth: 0 });
    const { nodeIds } = await cdp.send("DOM.querySelectorAll", { nodeId: root.nodeId, selector: "[data-glot-m]" });
    const out = [];
    for (const nodeId of nodeIds) {
      const { fonts } = await cdp.send("CSS.getPlatformFontsForNode", { nodeId });
      out.push([...fonts].sort((a, b) => b.glyphCount - a.glyphCount)[0] ?? { familyName: "" });
    }
    return out;
  } finally {
    await cdp.detach();
  }
}

/** Tightest budget wins when a key renders in several places. */
const capacity = (f: Fit) => f.max_width_px * (f.max_lines ?? 50);

const slugOf = (route: string) => route.replace(/[^\w]+/g, "_").replace(/^_|_$/g, "") || "index";

const RTL = new Set(["ar", "he", "fa", "ur"]);

async function grab(page: Page, url: string) {
  let res = await page.goto(url, { waitUntil: "load" });
  // Dev servers 500 transiently while compiling: one retry.
  if (res && res.status() >= 500) {
    await page.waitForTimeout(1500);
    res = await page.goto(url, { waitUntil: "load" });
  }
  // An error page has no overflow: reporting "0 issues" for it would be a lie.
  if (res && res.status() >= 400) throw new Error(`${url} → HTTP ${res.status()}`);
  // Dev servers keep HMR sockets/polls open: networkidle is best effort.
  await page.waitForLoadState("networkidle", { timeout: 5000 }).catch(() => {});
  // Scroll-reveal (IntersectionObserver fade-ins) stays blank until seen: walk the page down once, then back up.
  // ponytail: capped at 20000px / 5s, very long pages keep their bottom unrevealed.
  await page.evaluate(async () => {
    const end = Date.now() + 5000;
    for (let y = innerHeight; y < Math.min(document.documentElement.scrollHeight, 20000) && Date.now() < end; y += innerHeight) {
      scrollTo(0, y);
      await new Promise((r) => setTimeout(r, 100));
    }
    scrollTo(0, 0);
  });
  await page.waitForLoadState("networkidle", { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(300);
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
  if (o.harness) return runHarness(o);
  const shots = join(o.out, "screenshots");
  await mkdir(shots, { recursive: true });
  const browser = await launch();
  const report: Report = { generated_at: new Date().toISOString(), options: o, pages: [], issues: [] };
  const apiKey = process.env.ANTHROPIC_API_KEY;
  if (o.suggest && !apiKey) console.warn("--suggest: ANTHROPIC_API_KEY not set, skipping suggestions");
  const indexes: Record<string, Index> = o.catalogs ? await loadIndexes(o.catalogs, [o.sourceLocale, ...o.locales]) : {};
  const wanted = o.map === "*" ? null : new Set((o.map ?? "").split(",").map((k) => k.trim()).filter(Boolean));
  const seen = new Set<string>();
  const pad = 8;
  const fontsDir = o.fontsDir ?? join(o.out, "fonts");
  const fontSaves: Promise<void>[] = [];
  const measureKeys = async (page: Page, route: string, viewport: string) => {
    const probes = await page.evaluate(probe);
    const fonts = await platformFonts(page);
    report.layout ??= {};
    for (const [i, p] of probes.entries()) {
      // Dev marker (exact) first, else the rendered text (the first key with that text wins).
      const key = p.key ?? match(indexes[o.sourceLocale], p.text);
      if (!key || (o.map && wanted && !wanted.has(key))) continue;
      const f = fonts[i];
      const fit: Fit = {
        max_width_px: p.max_width_px,
        max_lines: p.max_lines,
        font: {
          family: f?.familyName || p.font.css.split(",")[0].replace(/["']/g, "").trim(),
          ...(f?.postScriptName ? { postscript: f.postScriptName } : {}),
          size_px: p.font.size_px, weight: p.font.weight, letter_spacing_px: p.font.letter_spacing_px,
          ...(p.font.text_transform ? { text_transform: p.font.text_transform } : {}),
        },
        text: p.ownText,
        ...(p.text_px !== undefined ? { text_px: p.text_px } : {}),
      };
      const occ: Occurrence = { route, viewport, selector: p.selector, max_width_px: p.max_width_px, max_lines: p.max_lines, by: p.key ? "marker" : "text" };
      const prev = report.layout[key];
      const occurrences = [...(prev?.occurrences ?? []), occ];
      report.layout[key] = !prev || capacity(fit) < capacity(prev) ? { ...fit, occurrences } : { ...prev, occurrences };
    }
  };
  const clipOf = (r: Rect, p = pad) => ({ x: Math.max(0, r.x - p), y: Math.max(0, r.y - p), width: r.width + 2 * p, height: r.height + 2 * p });
  // One crop per (locale, key): the first place it renders.
  const mapKeys = async (page: Page, els: El[], locale: string, route: string, vp: string, slug: string) => {
    if (!o.map) return;
    report.keys ??= [];
    for (const [i, e] of els.entries()) {
      const key = e.key ?? match(indexes[locale], e.text);
      if (!key || (wanted && !wanted.has(key)) || seen.has(locale + "\0" + key)) continue;
      seen.add(locale + "\0" + key);
      const crop = join("screenshots", `${slug}_key${i}.png`);
      await page.screenshot({ path: join(o.out, crop), clip: clipOf(e.rect, 24), fullPage: true });
      report.keys.push({ locale, route, viewport: vp, key, text: e.text, crop });
    }
  };

  try {
    for (const vp of o.viewports) {
      const [width, height] = vp.split("x").map(Number);
      // Reduced motion: reveal animations land on their final frame.
      const page = await browser.newPage({ viewport: { width, height }, reducedMotion: "reduce" });
      if (o.measure) {
        await mkdir(fontsDir, { recursive: true });
        page.on("response", (res) => {
          if (res.request().resourceType() !== "font") return;
          fontSaves.push(res.body().then(async (body) => {
            const sfnt = toSfnt(body);
            if (sfnt) await writeFile(join(fontsDir, fontFileName(sfnt)), sfnt);
          }).catch((e) => console.warn(`font ${res.url()}: ${e}`)));
        });
      }
      for (const route of o.routes) {
        // Locale-prefixed visits leave a locale cookie (next-intl NEXT_LOCALE) that would turn the source page into the last locale.
        await page.context().clearCookies();
        await grab(page, buildUrl(o, o.sourceLocale, route));
        await page.evaluate(unmark);
        const src = await page.evaluate(detect, { sourceTexts: null, checkRtl: false });
        const sourceTexts = src.texts;
        await mapKeys(page, src.els, o.sourceLocale, route, vp, `${o.sourceLocale}_${slugOf(route)}_${vp}`);
        if (o.measure) await measureKeys(page, route, vp);

        for (const locale of o.locales) {
          const url = buildUrl(o, locale, route);
          await grab(page, url);
          await page.evaluate(unmark);
          // The source locale is the reference: never "untranslated" against itself.
          const { found, els } = await page.evaluate(detect, { sourceTexts: locale === o.sourceLocale ? null : sourceTexts, checkRtl: RTL.has(locale.split("-")[0]) });
          const slug = `${locale}_${slugOf(route)}_${vp}`;
          const screenshot = join("screenshots", `${slug}.png`);
          await page.screenshot({ path: join(o.out, screenshot), fullPage: true });
          report.pages.push({ locale, route, viewport: vp, url, screenshot });

          await mapKeys(page, els, locale, route, vp, slug);

          for (const [i, { ctx, ...f }] of found.entries()) {
            const issue: Issue = { ...f, locale, route, viewport: vp };
            // Untranslated text is still in the source locale; overlaps carry "a ⨯ b".
            issue.key ??= match(indexes[f.kind === "untranslated" ? o.sourceLocale : locale], f.text.split(" ⨯ ")[0]);
            if (VISUAL.includes(f.kind) && f.rect) {
              issue.crop = join("screenshots", `${slug}_${i}_${f.kind}.png`);
              const png = await page.screenshot({ path: join(o.out, issue.crop), clip: clipOf(f.rect), fullPage: true });
              if (ctx) {
                issue.context_crop = join("screenshots", `${slug}_${i}_context.png`);
                await page.screenshot({ path: join(o.out, issue.context_crop), clip: clipOf(ctx), fullPage: true });
              }
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
    await Promise.all(fontSaves);
    await browser.close();
  }

  await writeFile(join(o.out, "report.json"), JSON.stringify(report, null, 2));
  await writeFile(join(o.out, "report.md"), toMarkdown(report));
  return report;
}

async function runHarness(o: Options): Promise<Report> {
  const fragments: Record<string, string> = JSON.parse(await readFile(o.harness!, "utf8"));
  const shots = join(o.out, "screenshots");
  await mkdir(shots, { recursive: true });
  const report: Report = { generated_at: new Date().toISOString(), options: { ...o, locales: Object.keys(fragments) }, pages: [], issues: [] };
  const browser = await launch();
  try {
    for (const vp of o.viewports) {
      const [width, height] = vp.split("x").map(Number);
      const page = await browser.newPage({ viewport: { width, height }, reducedMotion: "reduce" });
      const url = buildUrl(o, o.sourceLocale, o.routes[0] ?? "/");
      for (const [locale, html] of Object.entries(fragments)) {
        await grab(page, url);
        await page.evaluate(([h, lang, rtl]) => {
          document.documentElement.lang = lang;
          document.documentElement.dir = rtl ? "rtl" : "ltr";
          document.body.innerHTML = h;
        }, [html, locale, RTL.has(locale.split("-")[0]) ? "1" : ""] as const);
        await page.evaluate(() => document.fonts.ready);
        const { found } = await page.evaluate(detect, { sourceTexts: null, checkRtl: false });
        const slug = `${locale}_harness_${vp}`;
        const screenshot = join("screenshots", `${slug}.png`);
        await page.screenshot({ path: join(o.out, screenshot), fullPage: true });
        report.pages.push({ locale, route: "harness", viewport: vp, url, screenshot });
        for (const [i, { ctx: _ctx, ...f }] of found.entries()) {
          const issue: Issue = { ...f, locale, route: "harness", viewport: vp };
          if (VISUAL.includes(f.kind) && f.rect) {
            issue.crop = join("screenshots", `${slug}_${i}_${f.kind}.png`);
            await page.screenshot({ path: join(o.out, issue.crop), clip: { x: Math.max(0, f.rect.x - 8), y: Math.max(0, f.rect.y - 8), width: f.rect.width + 16, height: f.rect.height + 16 }, fullPage: true });
          }
          report.issues.push(issue);
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
