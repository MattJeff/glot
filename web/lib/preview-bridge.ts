/**
 * glot preview bridge: lets the locale matrix (/app/[slug]/preview) talk to an app shown in its iframes.
 * - posts `glot:ready` / `glot:height` (document height) to the parent;
 * - on `glot:pseudo {on}` pseudo-localizes every visible text node (accents + ~35% expansion + brackets).
 *
 * MUST stay self-contained: it is also shipped as a <script> snippet via `Function.prototype.toString`.
 */
export function installBridge() {
  if (window.parent === window || (window as unknown as { __glotBridge?: boolean }).__glotBridge) return;
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
  post({ type: "glot:ready" });
  height();
}

export const BRIDGE_SNIPPET = `<script>(${installBridge.toString()})()</script>`;
