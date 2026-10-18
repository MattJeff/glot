/**
 * glot preview bridge: lets the locale matrix (/app/[slug]/preview) talk to an app shown in its iframes.
 * - posts `glot:ready` / `glot:height` (document height) to the parent;
 * - on `glot:pseudo {on}` pseudo-localizes every visible text node (accents + ~35% expansion + brackets).
 * Live review (/app/[slug]/feedback, docs/design/live-review-contract.md):
 * - `glot:anchors {anchors: {id, key?, selector?, text?}[]}` → `glot:rects {rects: {[id]: Rect | null}, vw, vh}`, re-sent on scroll/resize/DOM change.
 *   Rects are viewport-relative (the parent overlays them on the iframe box).
 * - `glot:pick {on}` → hover outline; a click posts `glot:picked {selector, text, rect, sx, sy, vw}` (rect viewport-relative, sx/sy = scroll) instead of activating the page; Escape posts `glot:pick-cancel`.
 * - `glot:replace {key?, selector?, text?, value | null}` → live-previews `value` in the anchored element (null restores it).
 * - `glot:scroll {selector?, text?}` → scrolls the anchored element into view.
 *
 * MUST stay self-contained: it is also shipped as a <script> snippet via `Function.prototype.toString`.
 */
export function installBridge() {
  if (window.parent === window || (window as unknown as { __glotBridge?: boolean }).__glotBridge) return;
  // Pasted in <head>: wait for <body>.
  if (!document.body) return void document.addEventListener("DOMContentLoaded", installBridge);
  (window as unknown as { __glotBridge?: boolean }).__glotBridge = true;
  const MAP: Record<string, string> = {
    a: "á", b: "ƀ", c: "ç", d: "ð", e: "é", f: "ƒ", g: "ĝ", h: "ĥ", i: "î", j: "ĵ", k: "ķ", l: "ļ", m: "ɱ",
    n: "ñ", o: "ö", p: "þ", r: "ŕ", s: "š", t: "ţ", u: "û", w: "ŵ", y: "ý", z: "ž",
    A: "Å", C: "Ç", D: "Ð", E: "É", G: "Ĝ", I: "Î", N: "Ñ", O: "Ö", S: "Š", U: "Û", Y: "Ý", Z: "Ž",
  };
  const pseudo = (s: string) => {
    const core = s.trim();
    if (!core) return s;
    const out = core.replace(/[A-Za-z]/g, (c) => MAP[c] ?? c);
    return s.replace(core, () => `[${out} ${"·".repeat(Math.ceil(core.length * 0.35))}]`);
  };
  const originals = new Map<Text, string>();
  const applied = new Map<Text, string>();
  let on = false;
  const walk = () => {
    const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode() as Text | null; n; n = w.nextNode() as Text | null) {
      if (n.parentElement?.closest("script,style,code,pre,kbd,[translate=no]")) continue;
      if (applied.get(n) === n.nodeValue) continue;
      originals.set(n, n.nodeValue ?? "");
      const v = pseudo(n.nodeValue ?? "");
      applied.set(n, v);
      n.nodeValue = v;
    }
  };
  const restore = () => {
    originals.forEach((v, n) => {
      if (applied.get(n) === n.nodeValue) n.nodeValue = v;
    });
    originals.clear();
    applied.clear();
  };
  new MutationObserver(() => on && walk()).observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  const post = (msg: object) => window.parent.postMessage(msg, "*");
  const height = () => post({ type: "glot:height", height: document.documentElement.scrollHeight });
  new ResizeObserver(height).observe(document.body);
  window.addEventListener("message", (e) => {
    if (e.source !== window.parent || e.data?.type !== "glot:pseudo") return;
    on = !!e.data.on;
    if (on) walk();
    else restore();
  });

  // ── Live review ────────────────────────────────────────────────────────────
  type Anchor = { id?: string; selector?: string; text?: string; key?: string | null };
  const norm = (x: string) => x.replace(/\s+/g, " ").trim();
  const find = (a: Anchor): Element | null => {
    // Same order as the widget (live-review-contract §1): [data-glot-key] > selector > text search.
    let el: Element | null = null;
    try {
      el = (a.key ? document.querySelector(`[data-glot-key="${CSS.escape(a.key)}"]`) : null) ?? (a.selector ? document.querySelector(a.selector) : null);
    } catch {
      el = null;
    }
    const want = a.text ? norm(a.text) : "";
    if (el && (!want || norm(el.textContent ?? "").includes(want))) return el;
    if (!want) return el;
    // Fallback: smallest element whose own text matches (the page re-rendered or the selector drifted).
    const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) if (n.parentElement && norm(n.nodeValue ?? "").includes(want)) return n.parentElement;
    return el;
  };
  const rectOf = (el: Element) => {
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  };
  const selectorOf = (el: Element): string => {
    const parts: string[] = [];
    for (let e: Element | null = el; e && e !== document.body && parts.length < 8; e = e.parentElement) {
      if (e.id && !/\d{3,}|:/.test(e.id)) {
        parts.unshift(`#${CSS.escape(e.id)}`);
        break;
      }
      const tag = e.tagName.toLowerCase();
      const sib = e.parentElement ? Array.from(e.parentElement.children).filter((c) => c.tagName === e!.tagName) : [];
      parts.unshift(sib.length > 1 ? `${tag}:nth-of-type(${sib.indexOf(e) + 1})` : tag);
    }
    return parts.join(" > ");
  };
  let anchors: Anchor[] = [];
  let queued = false;
  const sendRects = () => {
    if (queued || !anchors.length) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const rects: Record<string, ReturnType<typeof rectOf> | null> = {};
      for (const a of anchors) {
        const el = find(a);
        rects[a.id ?? ""] = el ? rectOf(el) : null;
      }
      post({ type: "glot:rects", rects, vw: window.innerWidth, vh: window.innerHeight });
    });
  };
  window.addEventListener("scroll", sendRects, { capture: true, passive: true });
  window.addEventListener("resize", sendRects);
  new MutationObserver(sendRects).observe(document.documentElement, { childList: true, subtree: true, characterData: true });

  let picking = false;
  let hover: HTMLElement | null = null;
  let hoverOutline = "";
  const unhover = () => {
    if (hover) hover.style.outline = hoverOutline;
    hover = null;
  };
  const target = (e: Event) => {
    const el = e.target instanceof Element ? e.target : null;
    return el && el !== document.body && el !== document.documentElement ? (el as HTMLElement) : null;
  };
  document.addEventListener(
    "mouseover",
    (e) => {
      if (!picking) return;
      const el = target(e);
      if (el === hover) return;
      unhover();
      if (!el) return;
      hover = el;
      hoverOutline = el.style.outline;
      el.style.outline = "2px solid #10b981";
    },
    true,
  );
  document.addEventListener(
    "click",
    (e) => {
      if (!picking) return;
      const el = target(e);
      e.preventDefault();
      e.stopPropagation();
      if (!el) return;
      unhover();
      picking = false;
      const keyed = el.closest("[data-glot-key]");
      post({ type: "glot:picked", key: keyed?.getAttribute("data-glot-key") ?? null, selector: selectorOf(el), text: norm(el.innerText ?? el.textContent ?? "").slice(0, 500), rect: rectOf(el), sx: window.scrollX, sy: window.scrollY, vw: window.innerWidth });
    },
    true,
  );
  document.addEventListener("keydown", (e) => {
    if (!picking || e.key !== "Escape") return;
    picking = false;
    unhover();
    post({ type: "glot:pick-cancel" });
  });

  const replaced = new Map<Element, string>();
  window.addEventListener("message", (e) => {
    const d = e.data;
    if (e.source !== window.parent || typeof d?.type !== "string") return;
    if (d.type === "glot:anchors" && Array.isArray(d.anchors)) {
      anchors = d.anchors;
      sendRects();
    } else if (d.type === "glot:pick") {
      picking = !!d.on;
      if (!picking) unhover();
    } else if (d.type === "glot:replace") {
      replaced.forEach((v, el) => ((el as HTMLElement).innerText = v));
      replaced.clear();
      const el = typeof d.value === "string" ? find(d) : null;
      if (el) {
        replaced.set(el, (el as HTMLElement).innerText);
        (el as HTMLElement).innerText = d.value;
      }
      sendRects();
    } else if (d.type === "glot:scroll") {
      find(d)?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    }
  });

  post({ type: "glot:ready", features: ["anchors", "pick", "replace"] });
  height();
}

export const BRIDGE_SNIPPET = `<script>(${installBridge.toString()})()</script>`;
