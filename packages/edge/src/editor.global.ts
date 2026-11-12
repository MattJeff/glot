// glot website preview / click-to-edit engine → web/public/widget/glot-editor.js (IIFE, loaded by the dashboard
// only: never by a customer's site, not part of the tag's size budget).
// `glotEditor.render({ html, base, path, source, dir, tm, skip })` translates the source HTML of a customer page
// exactly as the tag will in a visitor's browser (same segmenter, hashes and writer: patcher.ts) and returns:
// - `srcdoc`: the page for an `<iframe sandbox="allow-scripts">` (opaque origin: no cookie, no access to the
//   dashboard). Every script of the site is removed and blocked by a CSP that only allows our click relay, which
//   posts `{ glot: "pick", segs: "3 7", rect, scroll }` to the parent when a text is clicked;
// - `segments`: what each index is (hash, context, source text, current translation, attribute);
// - `selectors(i)`: CSS selectors that would exclude the clicked text or a block around it, with how many
//   elements of the page each one matches — compound selectors only (`#id`, `tag.class`), what the server's
//   `protect::Selector` understands.
// ponytail: a static preview — text a site renders with JavaScript only is not shown here (the tag itself, in a
// real browser, translates it). A headless render (packages/renderer) is the upgrade.
import { domSegments, write, type PatcherTm } from "./patcher.ts";
import { lookup } from "./edge.ts";
import { check, template } from "./segment.ts";

type Opts = { html: string; base: string; path: string; source: string; dir?: string; tm: PatcherTm; skip?: string[] };
type Seg = { hash: string; context: string; source: string; value: string | null; attr?: string };

const RELAY = `addEventListener("click",function(e){e.preventDefault();e.stopPropagation();var t=e.target.closest&&e.target.closest("[data-glot-seg]");if(!t)return;var r=t.getBoundingClientRect();parent.postMessage({glot:"pick",segs:t.getAttribute("data-glot-seg"),rect:{x:r.left,y:r.top,w:r.width,h:r.height},scroll:scrollY},"*")},true);addEventListener("message",function(e){if(e.data&&e.data.glotScroll!=null)scrollTo(0,e.data.glotScroll)});addEventListener("submit",function(e){e.preventDefault()},true)`;
const esc = (s: string) => CSS.escape(s);
// Builder-generated names (`comp-kx3f9a2b`, `css-1q2w3e`) change on every publish: not a stable handle.
const stable = (s: string) => (s.match(/\d/g)?.length ?? 0) < 3;

function render(o: Opts) {
  const doc = new DOMParser().parseFromString(o.html, "text/html");
  doc.querySelectorAll("script,noscript,base,meta[http-equiv],link[rel=preload][as=script],link[rel=modulepreload]").forEach((e) => e.remove());
  const skip = (o.skip ?? []).filter((s) => { try { return doc.querySelector(s), true; } catch { return false; } }).map((s) => `,${s}`).join("");
  const ctx = template(o.path, o.tm.locale);
  const segments: Seg[] = [];
  const mine = new WeakMap<object, string>();
  for (const seg of domSegments(doc.documentElement, o.source, skip)) {
    const context = `${seg.role}|${ctx}`;
    const t = lookup(o.tm, seg.hash, context);
    const host = seg.el ?? seg.gaps.flat().find((n) => /\S/.test(n.nodeValue ?? ""))?.parentElement;
    const p = t === undefined ? undefined : check(seg, t);
    if (p) write(seg, p, mine);
    if (!host || !doc.body?.contains(host)) continue; // <title>, <meta>: edited from the review queue
    const i = segments.push({ hash: seg.hash, context, source: seg.text, value: t ?? null, attr: seg.attr }) - 1;
    host.setAttribute("data-glot-seg", `${host.getAttribute("data-glot-seg") ?? ""} ${i}`.trim());
  }
  doc.documentElement.lang = o.tm.locale;
  doc.documentElement.dir = o.dir === "rtl" ? "rtl" : "ltr";
  const nonce = Math.random().toString(36).slice(2);
  const head = doc.head ?? doc.documentElement;
  const el = (tag: string, a: Record<string, string>, text = "") => { const e = doc.createElement(tag); for (const k in a) e.setAttribute(k, a[k]); e.textContent = text; return e; };
  head.prepend(
    el("meta", { "http-equiv": "Content-Security-Policy", content: `script-src 'nonce-${nonce}'; object-src 'none'; form-action 'none'` }),
    el("base", { href: o.base }),
    el("style", {}, "[data-glot-seg]{cursor:pointer}[data-glot-seg]:hover{outline:2px solid #6d5efc;outline-offset:2px}"),
    el("script", { nonce }, RELAY),
  );
  /** Selectors excluding segment `i`'s element or one of its 3 nearest ancestors. */
  const selectors = (i: number) => {
    const out: { selector: string; count: number }[] = [];
    let e = doc.querySelector(`[data-glot-seg~="${i}"]`);
    for (let n = 0; e && e !== doc.body && n < 4; n++, e = e.parentElement) {
      const cls = [...e.classList].filter(stable).slice(0, 2);
      const selector = e.id && stable(e.id) ? `#${esc(e.id)}` : cls.length ? e.localName + cls.map((c) => `.${esc(c)}`).join("") : "";
      if (selector && !out.some((x) => x.selector === selector)) out.push({ selector, count: doc.querySelectorAll(selector).length });
    }
    return out;
  };
  return { srcdoc: `<!doctype html>${doc.documentElement.outerHTML}`, segments, selectors, translated: segments.filter((s) => s.value !== null).length };
}

(globalThis as unknown as { glotEditor: unknown }).glotEditor = { render };
