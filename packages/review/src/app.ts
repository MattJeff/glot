// The review widget (lazy-loaded, docs/design/live-review-contract.md). One Shadow DOM host; the only change
// made to the page itself is the in-place preview of a proposed text, restored on close.
import { CSS } from "./style";
import { EN, FR, type Labels, type ReviewOptions } from "./types";
import { capture, candidates, domPath, findAnchor, keyOf, placeholders, segments, selectorOf, textOf, textTarget } from "./dom";
import { ApiError, makeApi, type Anchor, type Ann, type Comment, type Context, type Issue, type Proposal, type Reaction, type Rephrased, type TextEntry } from "./api";

type El = HTMLElement;
type Kid = Node | string | null | false | undefined;

function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, any> = {}, ...kids: Kid[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k in el && !k.includes("-") && k !== "role" && k !== "list") (el as any)[k] = v;
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of kids) if (c != null && c !== false) el.append(c);
  return el;
}
const btn = (label: Kid | Kid[], onclick: (e: Event) => unknown, cls = "btn sm out", extra: Record<string, any> = {}) =>
  h("button", { className: cls, type: "button", onclick, ...extra }, ...([] as Kid[]).concat(label));

// Lucide paths (ISC), inline: no icon font, no request.
const ICONS = {
  msg: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
  list: "M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01",
  ext: "M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
  x: "M18 6 6 18M6 6l12 12",
  grip: "M9 5h.01M9 12h.01M9 19h.01M15 5h.01M15 12h.01M15 19h.01",
};
function ic(name: keyof typeof ICONS): Node {
  const s = document.createElement("span");
  s.className = "ic";
  s.setAttribute("aria-hidden", "true");
  s.innerHTML = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="${name === "grip" ? 3 : 2}" stroke-linecap="round" stroke-linejoin="round"><path d="${ICONS[name]}"/></svg>`;
  return s;
}
const EMOJI: Record<string, string> = { "+1": "👍", eyes: "👀", check: "✅" };
const TONES = ["shorter", "clearer", "formal", "warmer", "segment"] as const; // RV4 presets
const STATUSES = ["open", "in_progress", "resolved", "wontfix"] as const;
const done = (a: Ann) => a.status === "resolved" || a.status === "wontfix";

/** Route as the server computes it (contract §1): path, no trailing slash, leading locale segment dropped. */
export function routeOf(path: string, locale: string): string {
  let p = path.replace(/\/+$/, "") || "/";
  const seg = p.split("/")[1] || "";
  if (seg && seg.toLowerCase() === locale.toLowerCase()) p = p.slice(seg.length + 1) || "/";
  return p;
}

let started = false;

export function start(opts: ReviewOptions & { token: string }): void {
  if (started) return;
  started = true;
  const fr = /^fr\b/i.test(navigator.language);
  const L: Labels = { ...(fr ? FR : EN), ...opts.labels };
  const uiLang = opts.lang || (fr ? "fr" : "en");
  const uiDir = opts.dir || (/^(ar|he|fa|ur)\b/i.test(uiLang) ? "rtl" : "ltr");
  const NAME = "glot-review-name";
  const getName = () => { try { return localStorage.getItem(NAME) || undefined; } catch { return undefined; } };
  const api = makeApi(opts, () => (ctx?.me.user_id ? undefined : getName() || ctx?.me.name || undefined), () => {
    if (ctx && !ctx.me.user_id && !(getName() || ctx.me.name)) {
      pop.querySelector<El>("#nm")?.focus();
      throw new ApiError(400, "author_name_required", L.nameNeeded);
    }
  });
  const locale = () => opts.locale || document.documentElement.lang || "en";
  const dir = () => getComputedStyle(document.documentElement).direction;
  const reduced = () => matchMedia("(prefers-reduced-motion:reduce)").matches;

  // ---- shell ----
  const host = h("glot-review" as any, { style: "all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483646" });
  const root = host.attachShadow({ mode: "open" });
  const ui = h("div", { className: "ui", dir: uiDir, lang: uiLang });
  const live = h("div", { className: "sr", role: "status", "aria-live": "polite" });
  const say = (m: string) => { live.textContent = ""; setTimeout(() => (live.textContent = m), 30); };
  const cap = h("div", { className: "cap", tabIndex: 0, hidden: true, role: "application", "aria-label": L.pickHelp });
  const hl = h("div", { className: "hl", hidden: true });
  const tip = h("div", { className: "tip", hidden: true, dir: "ltr" });
  const pins = h("div");
  const pop = h("section", { className: "pop", role: "dialog", hidden: true });
  const panel = h("section", { className: "panel", hidden: true, "aria-label": L.list });
  root.append(h("style", {}, CSS), ui);
  document.documentElement.append(host);

  // ---- state ----
  let ctx: Context | null = null;
  let anns: Ann[] = [];
  let filter: string[] = ["open", "in_progress"];
  let picking: false | "annotate" | "pick" = false;
  let cands: El[] = [];
  let ki = -1;
  let opener: El | null = null;
  let target: El | null = null;
  let preview: { el: El; html: string } | null = null;
  let shownId: string | null = null;
  const anchors = new Map<string, El | null>();
  const canWrite = () => !!ctx && ctx.access !== "read";
  const canEdit = () => ctx?.access === "edit" || ctx?.access === "member";
  const mine = (a: { author: { user_id: string | null; link_id: string | null; name: string } }) =>
    !!ctx && (ctx.me.user_id ? a.author.user_id === ctx.me.user_id : a.author.link_id === ctx.me.link_id && a.author.name === (getName() || ctx.me.name));

  // ---- iframe protocol (§5): only when embedded by the glot Review page ----
  // Contract §5: data-app-origin; else the origin of data-app-url (the glot web app that embeds the site).
  let appOrigin = opts.appOrigin;
  try { appOrigin ||= opts.appUrl && new URL(opts.appUrl).origin; } catch {}
  const post = (type: string, data: object) => appOrigin && parent !== window && parent.postMessage({ type, ...data }, appOrigin);
  const here = () => ({ url: location.origin + location.pathname, route: routeOf(location.pathname, locale()), locale: locale() });

  // ---- toolbar ----
  const commentBtn = btn([ic("msg"), h("span", { className: "lbl" }, L.comment)], () => setPicking(picking ? false : "annotate"), "btn", { "aria-pressed": "false", "aria-label": L.comment, title: "Alt+Shift+R" });
  const count = h("span", { className: "cnt" }, "0");
  const listBtn = btn([ic("list"), h("span", { className: "lbl" }, L.list), count], () => togglePanel(), "btn", { "aria-expanded": "false" });
  const sel = h("select", { "aria-label": L.filter, onchange: () => { filter = sel.value.split(","); render(); } },
    h("option", { value: "open,in_progress" }, L.open), h("option", { value: "resolved,wontfix" }, L.resolved), h("option", { value: STATUSES.join(",") }, L.all));
  const saas = h("a", { className: "btn", target: "_blank", rel: "noopener", hidden: true, "aria-label": L.saas }, h("span", { className: "lbl" }, L.saas), ic("ext"));
  const grip = btn(ic("grip"), () => {}, "grip", { "aria-label": L.move, title: L.move });
  const bar = h("div", { className: "bar", role: "toolbar", "aria-label": L.bar },
    grip, commentBtn, listBtn, sel, h("span", { className: "loc", title: "locale", dir: "ltr" }, locale()), saas,
    btn(ic("x"), quit, "btn", { "aria-label": L.quit, title: L.quit }));
  ui.append(live, cap, hl, tip, pins, bar, panel, pop);

  // Drag (pointer) + arrow keys on the grip (WCAG 2.5.7 alternative). Position kept per browser.
  const POS = "glot-review-pos";
  let pos: { x: number; y: number } | null = null;
  try { pos = JSON.parse(localStorage.getItem(POS) || "null"); } catch {}
  const placeBar = () => {
    const w = bar.offsetWidth, hh = bar.offsetHeight;
    const x = pos ? pos.x : uiDir === "rtl" ? 16 : innerWidth - w - 16, y = pos ? pos.y : innerHeight - hh - 16;
    bar.style.left = Math.max(8, Math.min(x, innerWidth - w - 8)) + "px";
    bar.style.top = Math.max(8, Math.min(y, innerHeight - hh - 8)) + "px";
  };
  const savePos = () => { pos = { x: bar.offsetLeft, y: bar.offsetTop }; try { localStorage.setItem(POS, JSON.stringify(pos)); } catch {} };
  grip.addEventListener("pointerdown", (e) => {
    const dx = e.clientX - bar.offsetLeft, dy = e.clientY - bar.offsetTop;
    grip.setPointerCapture(e.pointerId);
    const mv = (m: PointerEvent) => { pos = { x: m.clientX - dx, y: m.clientY - dy }; placeBar(); };
    grip.addEventListener("pointermove", mv);
    grip.addEventListener("pointerup", () => { grip.removeEventListener("pointermove", mv); savePos(); }, { once: true });
  });
  grip.addEventListener("keydown", (e) => {
    const d = ({ ArrowLeft: [-24, 0], ArrowRight: [24, 0], ArrowUp: [0, -24], ArrowDown: [0, 24] } as Record<string, number[]>)[e.key];
    if (!d) return;
    e.preventDefault();
    pos = { x: bar.offsetLeft + d[0], y: bar.offsetTop + d[1] };
    placeBar(); savePos();
  });

  // ---- geometry ----
  function near(el: Element | null, node: El) {
    const w = node.offsetWidth || 380, hgt = node.offsetHeight || 300;
    if (!el) { node.style.left = (innerWidth - w) / 2 + "px"; node.style.top = Math.max(8, (innerHeight - hgt) / 3) + "px"; return; }
    const r = el.getBoundingClientRect();
    node.style.left = Math.max(8, Math.min(uiDir === "rtl" ? r.right - w : r.left, innerWidth - w - 8)) + "px";
    // Roomier side of the target; the popover scrolls inside rather than covering the text.
    const down = innerHeight - r.bottom - 16, up = r.top - 16, below = down >= hgt || down >= up;
    node.style.maxHeight = Math.max(200, Math.min(560, below ? down : up)) + "px";
    const hh = Math.min(hgt, parseFloat(node.style.maxHeight));
    node.style.top = Math.max(8, below ? Math.min(r.bottom + 8, innerHeight - hh - 8) : r.top - hh - 8) + "px";
  }
  function hover(el: El | null, selected = false) {
    if (!el) { hl.hidden = tip.hidden = true; return; }
    const r = el.getBoundingClientRect();
    Object.assign(hl.style, { left: r.left - 3 + "px", top: r.top - 3 + "px", width: r.width + 6 + "px", height: r.height + 6 + "px" });
    hl.className = "hl" + (selected ? " sel" : "");
    hl.hidden = false;
    tip.textContent = keyOf(el).key || L.noKey;
    tip.hidden = selected;
    tip.style.left = Math.max(4, Math.min(r.left, innerWidth - 200)) + "px";
    tip.style.top = (r.top > 30 ? r.top - 28 : r.bottom + 6) + "px";
  }

  // ---- comment (pick) mode: transparent overlay, so page links/buttons never fire ----
  function setPicking(on: typeof picking) {
    picking = on;
    commentBtn.setAttribute("aria-pressed", String(!!on));
    cap.hidden = !on;
    hover(null);
    if (on) { closePop(false); cands = candidates(host); ki = -1; cap.focus(); say(L.pickHelp); }
  }
  const under = (x: number, y: number) => textTarget(document.elementsFromPoint(x, y).find((e) => e !== host) || null);
  function chosen(el: El, pt?: { x: number; y: number }) {
    const mode = picking;
    setPicking(false);
    if (mode === "pick") post("glot:review:picked", { anchor: anchorOf(el, pt) });
    else openNew(el, commentBtn, pt);
  }
  cap.addEventListener("pointermove", (e) => e.pointerType === "mouse" && hover(under(e.clientX, e.clientY)));
  cap.addEventListener("click", (e) => { const el = under(e.clientX, e.clientY); el && chosen(el, { x: e.clientX, y: e.clientY }); });
  cap.addEventListener("wheel", () => hover(null), { passive: true });
  cap.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); if (picking === "pick") post("glot:pick-cancel", {}); setPicking(false); commentBtn.focus(); return; }
    if (e.key === "Tab" && cands.length) {
      e.preventDefault();
      ki = (ki + (e.shiftKey ? -1 : 1) + cands.length) % cands.length;
      const el = cands[ki];
      el.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
      requestAnimationFrame(() => hover(el));
      const k = keyOf(el).key;
      say(`${ki + 1}/${cands.length}: ${textOf(el).slice(0, 120)}${k ? ` (${k})` : ""}`);
    }
    if ((e.key === "Enter" || e.key === " ") && cands[ki]) { e.preventDefault(); chosen(cands[ki]); }
  });

  // Touch: long press on a text annotates it (no mode switch needed on a phone).
  let lp = 0, swallow = false;
  document.addEventListener("touchstart", (e) => {
    if (e.touches.length !== 1 || e.composedPath().includes(host) || !canWrite()) return;
    const t = e.touches[0];
    lp = window.setTimeout(() => {
      const el = under(t.clientX, t.clientY);
      if (el) { swallow = true; navigator.vibrate?.(10); getSelection()?.removeAllRanges(); openNew(el, commentBtn, { x: t.clientX, y: t.clientY }); }
    }, 550);
  }, { passive: true });
  const cancelLp = () => clearTimeout(lp);
  document.addEventListener("touchmove", cancelLp, { passive: true });
  document.addEventListener("touchend", cancelLp, { passive: true });
  const eat = (e: Event) => { if (swallow && !e.composedPath().includes(host)) { e.preventDefault(); e.stopPropagation(); if (e.type === "click") swallow = false; } };
  document.addEventListener("click", eat, true);
  document.addEventListener("contextmenu", eat, true);
  document.addEventListener("keydown", (e) => { if (e.altKey && e.shiftKey && e.code === "KeyR" && canWrite()) { e.preventDefault(); setPicking(picking ? false : "annotate"); } });

  function anchorOf(el: El, pt?: { x: number; y: number }): Anchor {
    const r = el.getBoundingClientRect();
    return {
      ...keyOf(el), selector: selectorOf(el), text: textOf(el).slice(0, 1000), dom: domPath(el), ...here(),
      viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
      rect: { x: r.left + scrollX, y: r.top + scrollY, w: r.width, h: r.height },
      point: pt && r.width && r.height ? { x: Math.min(1, Math.max(0, (pt.x - r.left) / r.width)), y: Math.min(1, Math.max(0, (pt.y - r.top) / r.height)) } : undefined,
    };
  }

  // ---- in-place preview (restored on close) ----
  function setPreview(el: El | null, text: string | null) {
    if (!el || text == null) return endPreview();
    if (preview && preview.el !== el) endPreview();
    if (!preview) preview = { el, html: el.innerHTML };
    el.textContent = text;
    schedule();
  }
  function endPreview() { if (preview) { const p = preview; preview = null; p.el.innerHTML = p.html; schedule(); } }

  // ---- popover ----
  function closePop(refocus = true) {
    endPreview();
    if (pop.hidden) return;
    pop.hidden = true;
    target = null; shownId = null;
    hover(null);
    if (refocus && opener?.isConnected) opener.focus();
  }
  pop.addEventListener("keydown", (e) => { if (e.key === "Escape") { e.preventDefault(); closePop(); } });
  function showPop(title: string, el: El | null, ...body: Kid[]) {
    pop.replaceChildren(
      h("div", { className: "head" }, h("h2", { className: "ttl", id: "pt" }, title), btn(ic("x"), () => closePop(), "btn sm", { "aria-label": L.close, title: L.close })),
      ...(body.filter(Boolean) as Node[]));
    pop.setAttribute("aria-labelledby", "pt");
    pop.hidden = false;
    target = el;
    if (el) hover(el, true);
    near(el, pop);
  }
  const focusFirst = () => (pop.querySelector<El>("[role=tab][aria-selected=true]") || pop.querySelector<El>("textarea,input,button.pri") || pop.querySelector<El>("button"))?.focus();
  const meta = (key: string | null | undefined) => h("div", { className: "row mut" },
    key ? h("code", { dir: "ltr" }, key) : h("span", {}, L.noKey), h("span", { className: "loc", dir: "ltr" }, locale()));
  const err = () => h("p", { role: "alert", className: "err" });
  const fail = (m: El, e: any) => (m.textContent = e?.status === 401 ? L.expired : e instanceof ApiError ? e.message : L.error);

  /** Tabs (ARIA tabs pattern: arrows move, Tab enters the panel). */
  function tabs(list: [string, () => Node][], first = 0): Node {
    const tl = h("div", { role: "tablist", className: "row" });
    const pnl = h("div", { role: "tabpanel", tabIndex: 0, id: "tp" });
    const bs = list.map(([label, make], i) => btn(label, () => pick(i), "btn sm", { role: "tab", id: "tb" + i, "aria-controls": "tp" }));
    function pick(i: number, focus = true) {
      bs.forEach((b, j) => { b.setAttribute("aria-selected", String(i === j)); b.tabIndex = i === j ? 0 : -1; b.classList.toggle("pri", i === j); });
      pnl.setAttribute("aria-labelledby", "tb" + i);
      endPreview();
      pnl.replaceChildren(list[i][1]());
      if (focus) bs[i].focus();
      near(target, pop);
    }
    tl.addEventListener("keydown", (e) => {
      const i = bs.indexOf(e.target as any), d = ({ ArrowRight: 1, ArrowLeft: -1 } as Record<string, number>)[e.key];
      if (i >= 0 && d) { e.preventDefault(); pick((i + (uiDir === "rtl" ? -d : d) + bs.length) % bs.length); }
    });
    tl.append(...bs);
    pick(first, false);
    return h("div", { className: "row", style: "flex-direction:column;align-items:stretch;gap:10px" }, tl, pnl);
  }

  /** Text field with placeholder highlighting + protection and live server validation (dry_run). */
  function textField(key: string | null, loc: string, value: string, label: string, onInput?: (v: string) => void) {
    const id = "f" + loc.replace(/\W/g, "");
    const ta = h("textarea", { id, value, dir: "auto", rows: 2, maxLength: 5000, spellcheck: true, lang: loc });
    const phs = placeholders(value);
    const mirror = h("div", { className: "mut", "aria-hidden": "true", dir: "auto" });
    const out = h("div", { id: id + "i", role: "status", "aria-live": "polite" });
    ta.setAttribute("aria-describedby", id + "i");
    let vt = 0, seq = 0;
    const local = (): Issue[] => {
      const now = placeholders(ta.value);
      const lost = phs.filter((p) => { const i = now.indexOf(p); if (i < 0) return true; now.splice(i, 1); return false; });
      return lost.length ? [{ severity: "error", message: `${L.phBroken} ${lost.join(" ")}` }] : [];
    };
    const show = (is: Issue[]) => {
      ta.setAttribute("aria-invalid", String(is.some((i) => i.severity === "error")));
      out.replaceChildren(is.length ? h("ul", {}, ...is.map((i) => h("li", { className: i.severity === "error" ? "err" : "wrn" }, "⚠ ", i.message)))
        : ta.value !== value ? h("span", { className: "okm" }, "✓ ", L.valid) : "");
    };
    const update = () => {
      mirror.replaceChildren(...segments(ta.value).map(([s, ph]) => (ph ? h("mark", {}, s) : s)));
      mirror.hidden = !placeholders(ta.value).length;
      onInput?.(ta.value);
      clearTimeout(vt);
      const l = local();
      show(l);
      if (l.length || !key || ta.value === value || !ta.value.trim()) return;
      const n = ++seq;
      vt = window.setTimeout(async () => {
        const r = await api.put(key, { [loc]: ta.value }, true).catch(() => null);
        if (r && n === seq) show(r.results[loc]?.issues || []);
      }, 300);
    };
    ta.addEventListener("input", update);
    update();
    const node = h("div", { className: "row", style: "flex-direction:column;align-items:stretch;gap:4px" },
      h("label", { htmlFor: id }, label), ta,
      phs.length ? h("div", { className: "mut" }, L.ph, " ", ...phs.map((p) => h("code", { dir: "ltr" }, p))) : null,
      mirror, out);
    return { node, ta, ok: () => !local().length, reset: () => { ta.value = value; update(); } };
  }

  /** Name for review-link writes (required by the server, kept per browser). */
  function nameField(): Node | null {
    if (!ctx || ctx.me.user_id || getName()) return null;
    const i = h("input", { id: "nm", required: true, maxLength: 80, autocomplete: "name", value: ctx.me.name || "",
      oninput: () => { try { localStorage.setItem(NAME, i.value.trim()); } catch {} } });
    try { if (i.value) localStorage.setItem(NAME, i.value); } catch {}
    return h("div", { className: "row" }, h("label", { htmlFor: "nm" }, L.yourName), i);
  }

  /** Comment + optional proposal (+ ✨ rewording, preview, validator). `ensure` returns the annotation to reword. */
  function composer(el: El | null, key: string | null, original: string, submit: (body: string, proposal?: string) => Promise<void>,
    ensure: (instruction: string) => Promise<Ann>, primary = L.send): Node {
    if (!canWrite()) return h("p", { className: "mut" }, L.readOnly);
    const body = h("textarea", { id: "cb", maxLength: 5000, rows: 3 });
    let proposal = original;
    const pv = h("input", { type: "checkbox", id: "cv", onchange: () => (pv.checked ? setPreview(el, proposal) : endPreview()) });
    const f = textField(key, locale(), original, L.proposal, (v) => { proposal = v; pv.checked && setPreview(el, v); });
    const msg = err();
    const seg = h("input", { id: "sg", placeholder: L.segmentHelp, maxLength: 120 });
    const props = h("div");
    /** AI proposals as radios: picking one fills the field and previews it in the page. */
    const showProposals = (a: Ann, r: Rephrased) => {
      const loc = locale();
      const choose = (p: Proposal) => { const v = p.variants[loc]?.value ?? p.text; f.ta.value = v; f.ta.dispatchEvent(new Event("input")); pv.checked = true; setPreview(el, v); };
      props.replaceChildren(r.proposals.length ? h("fieldset", { className: "props" }, h("legend", {}, L.proposals),
        ...r.proposals.map((p, i) => {
          const v = p.variants[loc], n = Object.values(p.variants).filter((x) => x.valid).length;
          const radio = h("input", { type: "radio", name: "pp", value: String(p.id), checked: i === 0, onchange: () => choose(p) });
          return h("label", { className: "prop" }, radio, h("span", {},
            h("span", { dir: "auto", style: "display:block" }, v?.value ?? p.text),
            h("span", { className: "mut", style: "display:block" }, p.explanation),
            ...(v?.issues || []).map((x) => h("span", { className: x.severity === "error" ? "err" : "wrn", style: "display:block" }, "⚠ ", x.message)),
            h("span", { className: p.valid ? "okm" : "wrn", style: "display:block;font-size:12px" }, p.valid ? "✓ " : "⚠ ", `${L.validIn} ${n}/${Object.keys(p.variants).length}`)));
        }),
        canEdit() && key ? h("div", { className: "row end" }, btn(L.applyAll, async () => {
          const id = Number((props.querySelector("input:checked") as HTMLInputElement | null)?.value ?? r.proposals[0].id);
          try { await api.applyRewrite(a.id, r.rewrite_id, id); endPreview(); await refresh(); closePop(); say(L.applied); } catch (x) { fail(msg, x); }
        }, "btn sm pri")) : null)
        : h("p", { className: "wrn" }, "⚠ ", r.rejected.map((x) => x.reason.join(", ")).join(" · ") || L.error));
      if (r.proposals[0]) choose(r.proposals[0]);
      props.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
      say(`${L.proposals} : ${r.proposals.length}`);
    };
    const det = h("details", {}, h("summary", { className: "mut" }, L.propose),
      h("div", { className: "row", style: "flex-direction:column;align-items:stretch;gap:6px;margin-top:6px" }, f.node,
        h("div", { className: "re", role: "group", "aria-label": L.rephrase },
          h("span", { "aria-hidden": "true" }, "✨"),
          ...TONES.map((t) => btn(L[t], async (e) => {
            const b = e.currentTarget as El;
            if (t === "segment" && !seg.value.trim()) { seg.focus(); return; }
            b.setAttribute("aria-busy", "true"); b.setAttribute("disabled", ""); say(L.loading); msg.textContent = "";
            try {
              const a = await ensure(`✨ ${L.rephrase} : ${L[t]}`);
              const r = await api.rephrase(a.id, t, t === "segment" ? seg.value.trim() : undefined);
              showProposals(a, r);
            } catch (x) { fail(msg, x); }
            b.removeAttribute("aria-busy"); b.removeAttribute("disabled");
          }))),
        props,
        h("div", { className: "row" }, h("label", { htmlFor: "sg", style: "font-weight:400" }, L.segmentLabel), seg),
        h("label", { className: "row", style: "font-weight:400" }, pv, L.preview)));
    const go = h("button", { className: "btn pri", type: "submit" }, primary);
    return h("form", { className: "row", style: "flex-direction:column;align-items:stretch;gap:8px", onsubmit: async (e: Event) => {
      e.preventDefault();
      const prop = proposal.trim() && proposal !== original ? proposal : undefined;
      if (!body.value.trim() && !prop) return body.focus();
      if (prop && !f.ok()) { det.open = true; return f.ta.focus(); }
      go.disabled = true; msg.textContent = "";
      try { await submit(body.value.trim() || L.proposal, prop); } catch (x) { fail(msg, x); go.disabled = false; }
    } }, nameField(), h("label", { htmlFor: "cb" }, L.body), body, det, msg,
      h("div", { className: "row end foot" }, btn(L.cancel, () => closePop()), go));
  }

  /** "Text" tab: manual edit of the current locale (save / submit / reset / revert / delete), or key creation. */
  function textTab(el: El | null, key: string | null, onKey: (k: string) => void, ann?: Ann): Node {
    const box = h("div", { className: "row", style: "flex-direction:column;align-items:stretch;gap:8px" }, h("p", { className: "mut" }, L.loading));
    const msg = err();
    if (!key) {
      const k = h("input", { id: "nk", dir: "ltr", maxLength: 200, placeholder: "pricing.cta" });
      box.replaceChildren(h("p", { className: "mut" }, L.noKeyHelp),
        canEdit() ? h("form", { className: "row", style: "flex-direction:column;align-items:stretch;gap:6px", onsubmit: async (e: Event) => {
          e.preventDefault();
          try {
            const t = await api.createKey(el ? textOf(el) : ann!.anchor.text, el ? anchorOf(el) : ann!.anchor, k.value.trim());
            say(`${L.keyCreated} : ${t.key}`); onKey(t.key);
          } catch (x) { fail(msg, x); }
        } }, nameField(), h("label", { htmlFor: "nk" }, L.keyName), k, msg, h("div", { className: "row end foot" }, h("button", { className: "btn pri", type: "submit" }, L.createKey)))
          : h("p", { className: "mut" }, L.noApply));
      return box;
    }
    api.text(key).then((t) => {
      const loc = locale(), cur = t.values[loc]?.status === "missing" ? "" : t.values[loc]?.value ?? "";
      let v = cur;
      const pv = h("input", { type: "checkbox", id: "tv", onchange: () => (pv.checked ? setPreview(el, v) : endPreview()) });
      const f = textField(key, loc, cur, `${L.tabText} (${loc})`, (x) => { v = x; pv.checked && setPreview(el, x); });
      const save = h("button", { className: "btn pri", type: "submit" }, canEdit() ? L.saveText : L.submit);
      let armed = false;
      const del = btn(L.unset, async () => {
        if (!armed) { armed = true; del.textContent = L.unsetConfirm; return; }
        try { await api.unset(key, loc); say(L.unsetDone); endPreview(); onKey(key); } catch (x) { fail(msg, x); }
      });
      box.replaceChildren(h("form", { className: "row", style: "flex-direction:column;align-items:stretch;gap:8px", onsubmit: async (e: Event) => {
        e.preventDefault();
        if (!f.ok() || v === cur || !v.trim()) return f.ta.focus();
        msg.textContent = "";
        try {
          if (canEdit()) { await api.put(key, { [loc]: v }); endPreview(); say(L.saved); onKey(key); }
          else {
            // Comment access: the proposal becomes a suggestion awaiting approval.
            const a = ann && mine(ann) ? await api.patch(ann.id, { suggestion: v }) : await api.create(el ? anchorOf(el) : ann!.anchor, L.proposal, v);
            say(L.submit); await refresh(); openAnn(a);
          }
        } catch (x: any) {
          const r = x?.details?.results?.[loc]?.issues as Issue[] | undefined;
          msg.textContent = r?.length ? r.map((i) => i.message).join(" · ") : (fail(msg, x), msg.textContent);
        }
      } },
        h("p", { className: "mut" }, `${L.source} (${t.source_locale}) : `, h("span", { dir: "auto" }, t.source)),
        nameField(), f.node, h("label", { className: "row", style: "font-weight:400" }, pv, L.preview), msg,
        h("div", { className: "row end foot" },
          btn(L.reset, () => { f.reset(); endPreview(); pv.checked = false; }),
          canEdit() ? btn(L.revert, async () => { try { await api.revert(key, loc); say(L.reverted); onKey(key); } catch (x) { fail(msg, x); } }) : null,
          canEdit() && cur ? del : null,
          canWrite() ? save : null)));
      near(target, pop);
    }, (x) => fail(box.firstChild as El, x));
    return box;
  }

  /** "All languages" tab: one field per locale, saved atomically (PUT /text/{key}). */
  function allTab(key: string, onKey: (k: string) => void): Node {
    const box = h("div", {}, h("p", { className: "mut" }, L.loading));
    api.text(key).then((t: TextEntry) => {
      const msg = err();
      const locs = [t.source_locale, ...ctx!.project.target_locales.filter((l) => l !== t.source_locale)];
      const fields = locs.map((l) => {
        const tr = t.values[l], cur = !tr || tr.status === "missing" ? "" : tr.value;
        if (l === t.source_locale) return { l, cur, node: h("p", {}, h("span", { className: "loc", dir: "ltr" }, l), " ", h("span", { className: "mut" }, L.source), h("br"), h("span", { dir: "auto", lang: l }, t.source)) } as any;
        const f = textField(key, l, cur, `${l}${cur ? "" : ` (${L.missing})`}`);
        return { l, cur, f, node: h("div", {}, f.node, canEdit() ? h("div", { className: "row end" },
          btn(L.revert, async () => { try { await api.revert(key, l); say(`${l} : ${L.reverted}`); onKey(key); } catch (x) { fail(msg, x); } }),
          cur ? btn(L.unset, async (e) => {
            const b = e.currentTarget as El;
            if (!b.dataset.armed) { b.dataset.armed = "1"; b.textContent = L.unsetConfirm; return; }
            try { await api.unset(key, l); say(`${l} : ${L.unsetDone}`); onKey(key); } catch (x) { fail(msg, x); }
          }) : null) : null) };
      });
      box.replaceChildren(h("form", { className: "row", style: "flex-direction:column;align-items:stretch;gap:10px", onsubmit: async (e: Event) => {
        e.preventDefault();
        const changed = fields.filter((x: any) => x.f && x.f.ta.value !== x.cur && x.f.ta.value.trim());
        const bad = changed.find((x: any) => !x.f.ok());
        if (bad) return bad.f.ta.focus();
        if (!changed.length) return;
        try { await api.put(key, Object.fromEntries(changed.map((x: any) => [x.l, x.f.ta.value]))); say(L.saved); onKey(key); }
        catch (x: any) {
          const res = x?.details?.results as Record<string, { issues: Issue[] }> | undefined;
          msg.textContent = res ? Object.entries(res).flatMap(([l, r]) => r.issues.filter((i) => i.severity === "error").map((i) => `${l} : ${i.message}`)).join(" · ") : (fail(msg, x), msg.textContent);
        }
      } }, nameField(), ...fields.map((x: any) => x.node), msg, canEdit() ? h("div", { className: "row end foot" }, h("button", { className: "btn pri", type: "submit" }, L.saveText)) : h("p", { className: "mut" }, L.noApply)));
      near(target, pop);
    }, () => box.replaceChildren(h("p", { className: "err", role: "alert" }, L.error)));
    return box;
  }

  async function openNew(el: El, from: El, pt?: { x: number; y: number }, tab = 0, keyOverride?: string) {
    opener = from;
    const { key: k0 } = keyOf(el), key = keyOverride || k0 || null, text = textOf(el);
    let draft: Ann | null = null;
    const create = async (body: string, suggestion?: string) => {
      const a = await api.create(anchorOf(el, pt), body, suggestion);
      post("glot:review:created", { annotation_id: a.id });
      endPreview();
      capture(el).then((shot) => shot && api.attach(a.id, shot).catch(() => {}));
      anchors.set(a.id, el);
      return a;
    };
    const reopen = (k: string) => openNew(el, from, pt, 1, k);
    showPop(L.newTitle, el, meta(key), h("q", { dir: "auto" }, text),
      tabs([
        [L.tabThread, () => composer(el, key, text, async (body, prop) => {
          const a = draft ? await api.patch(draft.id, { body, suggestion: prop ?? null }) : await create(body, prop);
          await refresh(); closePop(); say(`#${num(a)} ✓`);
        }, async (instr) => (draft ||= await create(instr)))],
        [L.tabText, () => textTab(el, key, reopen)],
        ...(key ? [[L.tabAll, () => allTab(key, (k: string) => openNew(el, from, pt, 2, k))]] as [string, () => Node][] : []),
      ], tab));
    focusFirst();
  }

  function reactions(a: Ann, rs: Reaction[] | undefined, cid?: string): Node[] {
    return Object.entries(EMOJI).map(([k, e]) => {
      const r = rs?.find((x) => x.emoji === k);
      return btn([h("span", { "aria-hidden": "true" }, e), r?.count ? " " + r.count : ""], () => act(() => api.react(a.id, k, cid)), "btn sm out",
        { "aria-pressed": String(!!r?.mine), "aria-label": `${k} (${r?.count || 0})`, disabled: !canWrite() });
    });
  }
  const md = (html: string, raw: string) => { const d = h("div", { dir: "auto" }); html ? (d.innerHTML = html) : (d.textContent = raw); return d; }; // body_html is sanitized by the server (contract §1)
  function commentView(a: Ann, c: Comment): Node {
    const li: El = h("li", { className: "cm" },
      h("div", { className: "row mut" }, h("strong", {}, mine(c) ? L.you : c.author.name), h("time", { dateTime: c.created_at }, ago(c.created_at))),
      md(c.body_html, c.body),
      h("div", { className: "row" }, ...reactions(a, (c as any).reactions, c.id),
        mine(c) ? btn(L.edit, () => editText(li, c.body, (v) => api.editComment(a.id, c.id, v), a), "btn sm") : null,
        mine(c) ? btn(L.del, () => act(() => api.delComment(a.id, c.id)), "btn sm") : null));
    return li;
  }
  function editText(li: El, value: string, save: (v: string) => Promise<unknown>, a: Ann) {
    const ta = h("textarea", { value, "aria-label": L.edit, maxLength: 5000 });
    li.replaceChildren(ta, h("div", { className: "row end" }, btn(L.cancel, () => openAnn(a)), btn(L.save, () => act(() => save(ta.value)), "btn sm pri")));
    ta.focus();
  }
  /** Run a mutation, refresh, keep the popover on the same annotation. */
  async function act(f: () => Promise<unknown>, doneMsg?: string) {
    const id = shownId;
    try { await f(); if (doneMsg) say(doneMsg); } catch (e: any) { say(e?.message || L.error); }
    await refresh().catch(() => {});
    const a = anns.find((x) => x.id === id);
    a ? openAnn(a) : closePop();
  }

  async function openAnn(a0: Ann, from?: El, tab = 0) {
    if (from) opener = from;
    post("glot:review:selected", { annotation_id: a0.id });
    const a = await api.get(a0.id).catch(() => a0);
    const el = anchors.get(a.id) || null;
    if (el) el.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
    const key = a.key;
    const statusCtl = canEdit()
      ? h("select", { "aria-label": L.filter, onchange: (e: Event) => act(() => api.patch(a.id, { status: (e.target as HTMLSelectElement).value })) },
        ...STATUSES.map((s) => h("option", { value: s, selected: s === a.status }, L["s_" + s as keyof Labels])))
      : h("span", { className: "badge" }, L[("s_" + a.status) as keyof Labels]);
    const thread = () => h("div", { className: "row", style: "flex-direction:column;align-items:stretch;gap:10px" },
      h("div", { className: "cm" },
        h("div", { className: "row mut" }, h("strong", {}, mine(a) ? L.you : a.author.name), h("time", { dateTime: a.created_at }, ago(a.created_at))),
        md(a.body_html, a.body),
        a.suggestion ? h("div", { className: "sg" }, h("span", { className: "mut" }, L.suggestion), h("p", { dir: "auto", style: "margin:0" }, a.suggestion),
          h("div", { className: "row" },
            el ? btn(L.preview, () => setPreview(el, preview ? null : a.suggestion), "btn sm out") : null,
            canEdit() && key ? btn(L.apply, () => act(() => api.apply(a.id), L.applied), "btn sm pri") : null,
            canWrite() ? btn(`${L.approve} (${a.approvals})`, () => act(() => api.react(a.id, "approve")), "btn sm out", { "aria-pressed": String(!!a.reactions.find((r) => r.emoji === "approve")?.mine) }) : null)) : null,
        h("div", { className: "row" }, ...reactions(a, a.reactions),
          mine(a) ? btn(L.edit, (e) => editText((e.currentTarget as El).closest(".cm") as El, a.body, (v) => api.patch(a.id, { body: v }), a), "btn sm") : null,
          mine(a) ? btn(L.del, () => act(() => api.del(a.id)), "btn sm") : null)),
      a.attachments[0] ? h("img", { className: "shot", src: api.shot(a.id, a.attachments[0].id), alt: "", loading: "lazy" }) : null,
      a.comments?.length ? h("ol", {}, ...a.comments.map((c) => commentView(a, c))) : null,
      composer(el, key, a.translation?.value ?? a.anchor.text, async (body, prop) => {
        if (prop) await api.patch(a.id, { suggestion: prop }).catch(() => api.comment(a.id, `${L.proposal} : ${prop}`));
        if (body !== L.proposal || !prop) await api.comment(a.id, body);
        await act(async () => {});
      }, async () => a, L.reply));
    shownId = a.id;
    showPop(`#${num(a)}`, el, h("div", { className: "row" }, meta(key), statusCtl),
      h("q", { dir: "auto" }, a.anchor.text),
      el ? null : h("p", { className: "wrn" }, "⚠ ", L.orphan),
      tabs([
        [L.tabThread, thread],
        [L.tabText, () => textTab(el, key, () => openAnn(a, undefined, 1), a)],
        ...(key ? [[L.tabAll, () => allTab(key, () => openAnn(a, undefined, 2))]] as [string, () => Node][] : []),
      ], tab));
    shownId = a.id;
    focusFirst();
  }

  // ---- pins & list ----
  /** Stable per-page numbers: creation order. */
  const num = (a: Ann) => [...anns].sort((x, y) => x.created_at.localeCompare(y.created_at)).findIndex((x) => x.id === a.id) + 1 || anns.length + 1;
  const shown = () => anns.filter((a) => filter.includes(a.status));
  function anchorAll() {
    const all = candidates(host);
    for (const a of anns) {
      const cur = anchors.get(a.id);
      if (!cur || !cur.isConnected || (preview?.el !== cur && textOf(cur) !== a.anchor.text && !a.key))
        anchors.set(a.id, findAnchor({ key: a.key, selector: a.anchor.selector, text: a.anchor.text }, all));
    }
  }
  function placePins() {
    for (const p of pins.children as HTMLCollectionOf<El>) {
      const el = anchors.get(p.dataset.id!);
      if (!el) continue;
      const rg = document.createRange();
      rg.selectNodeContents(el);
      const t = rg.getBoundingClientRect(), r = t.width ? t : el.getBoundingClientRect();
      p.style.left = (dir() === "rtl" ? r.left - 30 : r.right + 2) + "px";
      p.style.top = r.top - 16 + "px";
      p.hidden = r.bottom < 0 || r.top > innerHeight || (!r.width && !r.height);
    }
    if (target && !pop.hidden) hover(target, true);
  }
  function render() {
    const list = shown();
    count.textContent = String(list.length);
    listBtn.setAttribute("aria-label", `${L.list} (${list.length})`);
    pins.replaceChildren(...list.filter((a) => anchors.get(a.id)).map((a) => {
      const b = btn(String(num(a)), () => openAnn(a, b), "pin" + (done(a) ? " done" : ""), { "aria-label": `#${num(a)} ${L[("s_" + a.status) as keyof Labels]} : ${a.anchor.text.slice(0, 60)}` });
      b.dataset.id = a.id;
      return b;
    }));
    placePins();
    if (!panel.hidden) fillPanel();
  }
  function fillPanel() {
    const list = shown().sort((x, y) => num(x) - num(y));
    panel.replaceChildren(
      h("div", { className: "head" }, h("h2", { className: "ttl" }, `${L.list} (${list.length})`), btn(ic("x"), () => togglePanel(false), "btn sm", { "aria-label": L.close })),
      list.length ? h("ul", {}, ...list.map((a) => h("li", {}, btn([
        h("span", { className: "num", "aria-hidden": "true" }, String(num(a))),
        h("span", {},
          h("span", { className: "sr" }, `#${num(a)} `),
          h("span", { dir: "auto", style: "display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" }, a.anchor.text),
          h("span", { className: "mut" }, `${mine(a) ? L.you : a.author.name} · ${a.comment_count} 💬 · ${L[("s_" + a.status) as keyof Labels]}`),
          anchors.get(a.id) ? null : h("span", { className: "badge wrn", style: "display:inline-block;margin-inline-start:6px" }, L.orphan)),
      ], (e) => openAnn(a, e.currentTarget as El), "it")))) : h("p", { className: "mut" }, L.empty));
    const r = bar.getBoundingClientRect();
    panel.style.left = Math.max(8, Math.min(r.left, innerWidth - panel.offsetWidth - 8)) + "px";
    panel.style.top = (r.top > innerHeight / 2 ? Math.max(8, r.top - panel.offsetHeight - 8) : r.bottom + 8) + "px";
  }
  function togglePanel(on = panel.hidden) {
    panel.hidden = !on;
    listBtn.setAttribute("aria-expanded", String(on));
    if (on) { fillPanel(); panel.querySelector<El>(".it")?.focus(); } else listBtn.focus();
  }
  panel.addEventListener("keydown", (e) => e.key === "Escape" && togglePanel(false));

  let raf = 0;
  function schedule() { cancelAnimationFrame(raf); raf = requestAnimationFrame(placePins); }
  addEventListener("scroll", schedule, { capture: true, passive: true });
  addEventListener("resize", () => { placeBar(); schedule(); if (!pop.hidden) near(target, pop); });
  // DOM changes (re-render, SPA navigation): re-anchor, debounced. Our own preview is ignored.
  let mt = 0, lastPath = location.pathname;
  new MutationObserver(() => {
    clearTimeout(mt);
    mt = window.setTimeout(() => {
      if (location.pathname !== lastPath) { lastPath = location.pathname; anchors.clear(); closePop(false); post("glot:review:navigated", here()); refresh().catch(() => {}); return; }
      anchorAll(); render();
    }, 250);
  }).observe(document.body, { subtree: true, childList: true, characterData: true });

  // ---- data ----
  async function refresh() {
    anns = await api.list(routeOf(location.pathname, locale()), locale());
    anchorAll();
    render();
  }
  function ago(iso: string) {
    const s = (Date.parse(iso) - Date.now()) / 1000, rtf = new Intl.RelativeTimeFormat(uiLang, { numeric: "auto" });
    for (const [unit, n] of [["day", 86400], ["hour", 3600], ["minute", 60]] as const) if (Math.abs(s) >= n) return rtf.format(Math.round(s / n), unit);
    return rtf.format(0, "minute");
  }
  function quit() {
    try { sessionStorage.removeItem("glot-review"); } catch {}
    endPreview(); es?.close(); host.remove(); started = false;
  }

  // Page → widget (§5).
  addEventListener("message", (e) => {
    if (!appOrigin || e.origin !== appOrigin || e.source !== parent || parent === window || typeof e.data?.type !== "string") return;
    const d = e.data;
    if (d.type === "glot:review:focus") { const a = anns.find((x) => x.id === d.annotation_id); a && openAnn(a); }
    if (d.type === "glot:review:mode") { const m = d.pick ? "pick" : d.annotate ? "annotate" : false; if (m !== picking) setPicking(m); }
    if (d.type === "glot:review:filter" && Array.isArray(d.status)) { filter = d.status; render(); }
    if (d.type === "glot:review:preview") {
      const el = (d.annotation_id && anchors.get(d.annotation_id)) || (d.anchor && findAnchor(d.anchor, candidates(host))) || null;
      d.value == null ? endPreview() : setPreview(el || null, d.value);
    }
  });

  let es: EventSource | null = null;
  (async () => {
    try {
      ctx = await api.context();
    } catch (e: any) {
      bar.replaceChildren(grip, h("p", { className: "err", role: "alert", style: "margin:0;padding:0 8px" }, e?.status === 401 ? L.expired : L.error), btn(ic("x"), quit, "btn", { "aria-label": L.quit }));
      return placeBar();
    }
    commentBtn.hidden = !canWrite();
    if (opts.appUrl) { saas.href = `${opts.appUrl.replace(/\/$/, "")}/projects/${encodeURIComponent(ctx.project.slug)}/review?route=${encodeURIComponent(routeOf(location.pathname, locale()))}`; saas.hidden = false; }
    placeBar();
    await refresh().catch(() => say(L.error));
    post("glot:review:ready", here());
    es = api.events((ev) => {
      // Others' changes: refetch; never re-render under the reviewer's typing.
      const typing = root.activeElement?.matches("textarea,input");
      refresh().then(() => {
        if (ev.kind === "annotation.created" && !(ctx!.me.user_id ? ev.actor.user_id === ctx!.me.user_id : ev.actor.link_id === ctx!.me.link_id)) say(`${L.added} ${ev.actor.name}`);
        const a = shownId && ev.annotation_id === shownId && anns.find((x) => x.id === shownId);
        if (a && !typing) openAnn(a);
      }).catch(() => {});
    });
  })();
  placeBar();
}
