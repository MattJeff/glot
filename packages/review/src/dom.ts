// Page-side helpers: which element is "a text", which key it renders, how to find it again.

// Dev key markers (`@glot/runtime/dev`): U+2063, key bytes as variation selectors, U+2063.
// Copied from packages/visual-qa/src/keys.ts (decodeKeys) to stay dependency-free.
const MARK = /⁣([\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]+)⁣/gu;

export function decodeKeys(text: string): { keys: string[]; text: string } {
  const keys: string[] = [];
  const clean = text.replace(MARK, (_, bytes: string) => {
    const b = Array.from(bytes, (c) => { const p = c.codePointAt(0)!; return p < 0xe0100 ? p - 0xfe00 : p - 0xe0100 + 16; });
    keys.push(new TextDecoder().decode(new Uint8Array(b)));
    return "";
  });
  return { keys, text: clean };
}

export const norm = (s: string) => decodeKeys(s).text.replace(/\s+/g, " ").trim();

/** Visible text of an element, markers stripped. */
export const textOf = (el: Element) => norm((el as HTMLElement).innerText ?? el.textContent ?? "");

/** Has a non-blank text node of its own. */
const ownText = (el: Element) => Array.from(el.childNodes).some((n) => n.nodeType === 3 && norm(n.nodeValue || ""));

const SKIP = /^(script|style|noscript|template|svg|html|body|head|iframe|canvas|video|audio|textarea|select)$/;

/** Element holding the text under the pointer: climbs out of inline wrappers (<b> in a sentence). */
export function textTarget(el: Element | null): HTMLElement | null {
  for (let i = 0; el && i < 6; el = el.parentElement, i++) {
    if (SKIP.test(el.localName)) return null;
    if (ownText(el) || el.matches("input[type=submit],input[type=button],[placeholder]")) {
      while (el.parentElement && getComputedStyle(el).display === "inline" && !SKIP.test(el.parentElement.localName)) el = el.parentElement;
      const t = textOf(el);
      return t && t.length <= 1000 ? (el as HTMLElement) : null;
    }
  }
  return null;
}

/** Key rendered by `el` and how we know: [data-glot-key] (closest), else a dev marker. */
export function keyOf(el: Element): { key?: string; key_source?: "attribute" | "marker" } {
  const a = el.closest<HTMLElement>("[data-glot-key]")?.dataset.glotKey;
  if (a) return { key: a, key_source: "attribute" };
  const k = decodeKeys(el.textContent || "").keys[0];
  return k ? { key: k, key_source: "marker" } : {};
}

const stableId = (id: string) => id && !/\d{3,}|:|^radix-|^headlessui-/.test(id);

/** Robust selector (contract §1): #id stable > [data-testid] > [data-glot-key] > tag:nth-of-type path from the closest stable ancestor. */
export function selectorOf(el: Element): string {
  const p: string[] = [];
  for (let e: Element | null = el; e && e !== document.body && e !== document.documentElement; e = e.parentElement) {
    const q = (a: string) => `[${a}="${CSS.escape(e!.getAttribute(a)!)}"]`;
    if (stableId(e.id)) { p.unshift("#" + CSS.escape(e.id)); break; }
    if (e.hasAttribute("data-testid")) { p.unshift(q("data-testid")); break; }
    if (e.hasAttribute("data-glot-key")) { p.unshift(q("data-glot-key")); break; }
    const sib = e.parentElement ? Array.from(e.parentElement.children).filter((c) => c.localName === e!.localName) : [];
    p.unshift(e.localName + (sib.length > 1 ? `:nth-of-type(${sib.indexOf(e) + 1})` : ""));
  }
  return p.join(">").slice(0, 1000);
}

/** Tag path (no text), like @glot/feedback: `main>section#pricing>h2`. */
export function domPath(el: Element): string {
  const p: string[] = [];
  for (let e: Element | null = el; e && e !== document.body && p.length < 6; e = e.parentElement) p.unshift(e.localName + (e.id ? "#" + e.id : ""));
  return p.join(">").slice(0, 300);
}

const visible = (el: Element) => {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== "hidden";
};

/** Every visible text element of the page, document order (keyboard selection mode). */
export function candidates(host: Element): HTMLElement[] {
  const out = new Set<HTMLElement>();
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) {
    if (!norm(n.nodeValue || "") || host.contains(n)) continue;
    const t = textTarget(n.parentElement);
    if (t && visible(t)) out.add(t);
  }
  return [...out];
}

type Anchor = { key?: string | null; selector?: string | null; text?: string | null };

/** Re-anchor (contract §1): key whose text ≈ text > selector > text search. null = orphan. */
export function findAnchor(a: Anchor, all: HTMLElement[]): HTMLElement | null {
  if (a.key) {
    const byKey = all.filter((e) => keyOf(e).key === a.key);
    const e = byKey.find((e) => textOf(e) === a.text) || byKey[0];
    if (e) return e;
  }
  let sel: HTMLElement | null = null;
  try { sel = a.selector ? document.querySelector<HTMLElement>(a.selector) : null; } catch {}
  if (sel && !visible(sel)) sel = null;
  if (sel && textOf(sel) === a.text) return sel;
  // Same place, edited text (e.g. a suggestion was just applied) beats nothing.
  return (a.text && all.find((e) => textOf(e) === a.text)) || sel;
}

/** ICU arguments and tags of a message, top level: `{name}`, `{count, plural, …}`, `<b>`. Order-free. */
export function placeholders(s: string): string[] {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "{") {
      let d = 0, j = i;
      for (; j < s.length; j++) if (s[j] === "{") d++; else if (s[j] === "}" && --d === 0) break;
      out.push("{" + s.slice(i + 1, j).split(",")[0].trim() + "}");
      i = j;
    } else if (s[i] === "<") {
      const m = /^<\/?[\w.-]+\s*\/?>/.exec(s.slice(i));
      if (m) { out.push(m[0].replace(/\s/g, "")); i += m[0].length - 1; }
    }
  }
  return out.sort();
}

/** Segments of `s` with placeholders flagged (for highlighting). */
export function segments(s: string): [string, boolean][] {
  return s.split(/(\{(?:[^{}]|\{[^{}]*\})*\}|<\/?[\w.-]+\s*\/?>)/).filter(Boolean).map((x) => [x, /^[{<]/.test(x)]);
}

/**
 * Cropped picture of `el` and its surroundings: the closest block ancestor, serialized with computed styles
 * into an SVG <foreignObject>, painted on a canvas, target outlined. Best effort: null when tainted / too big.
 */
export async function capture(el: HTMLElement): Promise<Blob | null> {
  try {
    let box: HTMLElement = el;
    for (let e = el.parentElement, i = 0; e && e !== document.body && i < 3; e = e.parentElement, i++) {
      if (e.getBoundingClientRect().width > 800) break;
      box = e;
    }
    const r = box.getBoundingClientRect(), t = el.getBoundingClientRect();
    const w = Math.ceil(Math.min(r.width, 1000)), h = Math.ceil(Math.min(r.height, 4000));
    if (w < 4 || h < 4) return null;
    // Crop: the target ± 64 px of context, full width of its block.
    const y0 = Math.max(0, Math.floor(t.top - r.top - 64)), ch = Math.min(h - y0, Math.ceil(t.height + 128));
    if (ch < 4) return null;
    const clone = box.cloneNode(true) as HTMLElement;
    const src = [box, ...box.querySelectorAll("*")], dst = [clone, ...clone.querySelectorAll("*")];
    // ponytail: inline every computed property on ≤ 400 nodes; larger blocks are cropped out instead of serialized.
    if (src.length > 400) return null;
    src.forEach((s, i) => {
      const cs = getComputedStyle(s);
      let css = "";
      for (let j = 0; j < cs.length; j++) css += `${cs[j]}:${cs.getPropertyValue(cs[j])};`;
      (dst[i] as HTMLElement).setAttribute("style", css);
    });
    clone.style.margin = "0";
    const bg = getComputedStyle(document.body).backgroundColor;
    const xml = new XMLSerializer().serializeToString(clone);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><foreignObject width="100%" height="100%"><div xmlns="http://www.w3.org/1999/xhtml" style="background:${bg === "rgba(0, 0, 0, 0)" ? "#fff" : bg}">${xml}</div></foreignObject></svg>`;
    const img = new Image();
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
    await img.decode();
    const dpr = Math.min(2, devicePixelRatio || 1);
    const c = document.createElement("canvas");
    c.width = w * dpr; c.height = ch * dpr;
    const g = c.getContext("2d")!;
    g.scale(dpr, dpr);
    g.drawImage(img, 0, -y0);
    g.strokeStyle = "#e11d48"; g.lineWidth = 2;
    g.strokeRect(t.left - r.left - 3, t.top - r.top - y0 - 3, t.width + 6, t.height + 6);
    return await new Promise((ok) => c.toBlob(ok, "image/png"));
  } catch {
    return null;
  }
}
