// glot Edge client patcher: keeps an Edge-translated page translated after the app takes over.
// SPAs re-render (Leptos/Svelte mounts, React client-side recovery, route changes, late fetches) with the text
// of their bundle, i.e. the source language. A MutationObserver re-translates every changed block in the same
// task as the change: records are delivered at the microtask checkpoint, before the next paint (no flash).
// - Lean DOM segmenter: walks the LIVE nodes of a block with the segmenter's rules (segment.ts: blocks/inline,
//   roles, `<0>…</0>` / `<1/>` / `{0}` placeholders, protect/mask) — same segment text, same hash as the server,
//   without shipping the HTML tokenizer. Text nodes are rewritten in place (nodeValue / attribute), never replaced:
//   framework-owned nodes, listeners and refs survive. A translation whose tag order differs is not applied.
// - Version: the page says `<meta name="glot-tm" content="es@12">`; a TM older than 12 is ignored, so a page
//   never mixes two versions. TM sources: inline `<script type="application/json" id="glot-tm">`, else `tmUrl`
//   (last good copy kept in localStorage: used when glot is unreachable, if not older).
// - Discovery: text the app renders on the client (after boot) that the TM lacks is reported to `missing`
//   (same-origin `/__glot/missing`, the runtime adds the key and the session flag): batched, debounced, once per
//   page view, never with a session cookie, never personal data (segment.ts protect). Then the TM is re-polled
//   (30 s, 2 min, 10 min) and the new version applied in place.
// - `[data-glot-prerender]` (renderer snapshot of an app that appends to <body>) is removed when the app adds
//   its first own element.
import { mask, collapseWs, hasLetter, looksLikeAddress, isPii, stripPlaceholders, check, hash16, template, allWs, INLINE, VOID, MEDIA, ROLE_INLINE, META_NAMES, CODE_TAGS, type Value, type Part } from "./segment.ts";
import { lookup, SESSION_COOKIE, type Tm } from "./edge.ts";
export { hash16, template } from "./segment.ts";

/** `skip` = the project's excluded selectors (sent along with the TM by the JS runtimes). */
export type PatcherTm = Tm & { skip?: string[] };

const ATTRS = ["title", "alt", "aria-label", "aria-description", "placeholder", "value", "content"];
const OPAQUE = "[translate=no i],.notranslate,[data-glot-skip],[data-glot-user],[contenteditable]:not([contenteditable=false i])";
const MUTED = `${CODE_TAGS},textarea,select,noscript,${OPAQUE}`;

// ------------------------------------------------------------------------------------------- DOM segmenter

type It =
  | { t: 0; n: Text; s: string } // text
  | { t: 1; e: Element; r?: string } // inline open (r = its role, for a/button/label/[role])
  | { t: 2; e: Element } // inline close
  | { t: 3; e: Element } // void / opaque inline
  | { t: 4 }; // comment
type Tok = { p: "open" | "close" | "void"; n: number; e: Element };
export interface DomSegment {
  text: string;
  role: string;
  hash: string;
  discoverable: boolean;
  values: Value[];
  tags: { pair: boolean }[];
  /** Text segment: placeholders in order + the text nodes between them. */
  toks: Tok[];
  gaps: Text[][];
  /** Attribute segment: element + attribute name. */
  el?: Element;
  attr?: string;
}

const trimmable = (it: It) => (it.t === 0 ? allWs(it.s) : it.t > 2);
const roleOf = (e: Element) => (e.getAttribute("role") ?? "").trim().toLowerCase() || e.localName;

/** Segments of `root` (and its descendants), as segment.ts would cut the same markup. */
export function domSegments(root: Element, source: string, skip = ""): DomSegment[] {
  const out: DomSegment[] = [];
  let run: It[] = [];
  const roles: string[] = [];
  const make = (text: string, values: Value[], raw: string, role: string): DomSegment | undefined => {
    if (!hasLetter(stripPlaceholders(text))) return;
    return { text, role, hash: hash16(`${source}\u0000${text}`), discoverable: !values.some((v) => isPii(v.kind)) && !looksLikeAddress(raw), values, tags: [], toks: [], gaps: [[]] };
  };
  const emit = (s: It[], role: string) => {
    let text = "", raw = "", pend = "";
    const values: Value[] = [], tags: { pair: boolean }[] = [], toks: Tok[] = [], gaps: Text[][] = [[]], ids = new Map<Element, number>();
    const fl = () => { if (pend) raw += pend + " ", text += mask(pend, values, []), (pend = ""); };
    for (const it of s) {
      if (it.t === 0) { pend += it.s; gaps[gaps.length - 1].push(it.n); continue; }
      fl();
      raw += " ";
      if (it.t === 4) continue;
      const n = it.t === 2 ? ids.get(it.e)! : tags.length;
      if (it.t !== 2) tags.push({ pair: it.t === 1 });
      if (it.t === 1) ids.set(it.e, n);
      text += it.t === 1 ? `<${n}>` : it.t === 2 ? `</${n}>` : `<${n}/>`;
      toks.push({ p: it.t === 1 ? "open" : it.t === 2 ? "close" : "void", n, e: it.e });
      gaps.push([]);
    }
    fl();
    const seg = make(collapseWs(text), values, raw, role);
    if (seg) out.push(Object.assign(seg, { tags, toks, gaps }));
  };
  const piece = (s: It[], role: string): void => {
    for (;;) {
      while (s.length && trimmable(s[0])) s = s.slice(1);
      while (s.length && trimmable(s[s.length - 1])) s = s.slice(0, -1);
      const f = s[0], l = s[s.length - 1];
      if (s.length < 2 || f.t !== 1 || l.t !== 2 || f.e !== l.e) break;
      role = f.r ?? role;
      s = s.slice(1, -1);
    }
    if (!s.length) return;
    let depth = 0, loose = false, spaced = false, allRoles = true, g = 0;
    const groups: [number, number][] = [];
    s.forEach((it, i) => {
      if (it.t === 1) { if (!depth++) (g = i), (allRoles &&= it.r !== undefined); }
      else if (it.t === 2) { depth = Math.max(0, depth - 1); if (!depth) groups.push([g, i + 1]); }
      else if (it.t === 0 && !depth) { const ws = allWs(it.s); loose ||= !ws; spaced ||= ws; }
    });
    if (!loose && (allRoles || !spaced)) for (const [a, b] of groups) piece(s.slice(a, b), role);
    else emit(s, role);
  };
  const flush = () => {
    if (!run.length) return;
    // Empty inline elements (`<i class="icon"></i>`) are voids; unmatched open/close split the run.
    const items: It[] = [];
    for (const it of run) {
      if (it.t === 2) {
        const last = items[items.length - 1];
        const at = items.length - (last?.t === 0 && allWs(last.s) ? 1 : 0);
        const o = items[at - 1];
        if (o?.t === 1 && o.e === it.e) { items.length = at - 1; items.push({ t: 3, e: it.e }); continue; }
      }
      items.push(it);
    }
    run = [];
    const role = roles[roles.length - 1] ?? "body";
    const open = new Set<Element>(), both = new Set<Element>();
    for (const it of items) if (it.t === 1) open.add(it.e); else if (it.t === 2 && open.has(it.e)) both.add(it.e);
    let start = 0;
    items.forEach((it, i) => { if ((it.t === 1 || it.t === 2) && !both.has(it.e)) piece(items.slice(start, i), role), (start = i + 1); });
    piece(items.slice(start), role);
  };
  const attrs = (e: Element, name: string) => {
    for (const a of ATTRS) {
      const v = e.getAttribute(a);
      let role = `${name}:${a}`;
      if (v === null) continue;
      if (a === "value") { if (name !== "input" || !["button", "submit", "reset"].includes((e.getAttribute("type") ?? "-").toLowerCase())) continue; }
      else if (a === "content") {
        const m = (e.getAttribute("name") ?? e.getAttribute("property"))?.toLowerCase();
        if (name !== "meta" || !m || !META_NAMES.includes(m)) continue;
        role = `meta:${m}`;
      }
      const values: Value[] = [];
      const seg = make(collapseWs(mask(v, values, [])), values, v, role);
      if (seg) out.push(Object.assign(seg, { values, el: e, attr: a }));
    }
  };
  const visit = (n: Node, lvl: number): void => {
    if (n.nodeType === 3) return void (lvl || run.push({ t: 0, n: n as Text, s: n.nodeValue! }));
    if (n.nodeType === 8) return void (lvl || run.push({ t: 4 }));
    if (n.nodeType !== 1) return;
    const e = n as Element, name = e.localName;
    // 3 = opaque (translate=no, user content, project selectors), 2 = form field, 1 = code
    const sk = lvl === 2 ? 0 : e.matches(OPAQUE + skip) ? 3 : CODE_TAGS.includes(name) || name === "noscript" ? 1 : name === "textarea" || name === "select" ? 2 : 0;
    if (lvl !== 2 && sk < 3 && sk !== 1) attrs(e, name);
    const level = Math.max(lvl, sk ? 2 : MEDIA.includes(name) ? 1 : 0);
    const kids = () => { for (let c = e.firstChild; c; c = c.nextSibling) visit(c, level); };
    if (lvl) return kids();
    if (INLINE.includes(name)) {
      if (level || VOID.includes(name)) return run.push({ t: 3, e }), kids();
      run.push({ t: 1, e, r: ROLE_INLINE.includes(name) || e.hasAttribute("role") ? roleOf(e) : undefined });
      kids();
      return void run.push({ t: 2, e });
    }
    flush();
    if (level) return kids();
    roles.push(roleOf(e));
    kids();
    flush();
    roles.pop();
  };
  visit(root, 0);
  flush();
  return out;
}

/** Write `parts` (a checked translation) into the live nodes of `seg`; false if its placeholder order differs. */
function write(seg: DomSegment, parts: Part[], mine: WeakMap<object, string>): boolean {
  const val = (x: Part) => (x.p === "text" ? x.s : x.p === "val" ? seg.values[x.n].value : "");
  if (seg.el) {
    const v = parts.map(val).join("");
    if (seg.el.getAttribute(seg.attr!) !== v) seg.el.setAttribute(seg.attr!, v);
    return true;
  }
  const want = [""];
  let k = 0;
  for (const x of parts) {
    if (x.p === "text" || x.p === "val") want[want.length - 1] += val(x);
    else {
      const t = seg.toks[k++];
      if (!t || t.p !== x.p || t.n !== x.n) return false;
      want.push("");
    }
  }
  if (k !== seg.toks.length) return false;
  seg.gaps.forEach((g, i) => {
    const cur = g.map((n) => n.nodeValue).join("");
    let w = want[i];
    if (!i) w = /^\s*/.exec(cur)![0] + w; // the edges' whitespace stays (spacing with the next inline)
    if (i === seg.gaps.length - 1) w += /\s*$/.exec(cur)![0];
    if (cur === w) return;
    if (g.length) g.forEach((n, j) => { const v = j ? "" : w; if (n.nodeValue !== v) n.nodeValue = v; mine.set(n, v); });
    else if (/\S/.test(w)) {
      const t = document.createTextNode(w), next = seg.toks[i], prev = seg.toks[i - 1];
      mine.set(t, w);
      if (next) next.p === "close" ? next.e.append(t) : next.e.before(t);
      else prev?.e.after(t);
    }
  });
  return true;
}

// ------------------------------------------------------------------------------------------- observer

/** Nearest block ancestor-or-self of a changed node (`<meta>` itself): domSegments() of it covers the subtree. */
function block(n: Node): Element | null {
  let e = n.nodeType === 1 ? (n as Element) : n.parentElement;
  while (e && e.localName !== "meta" && INLINE.includes(e.localName)) e = e.parentElement;
  return e;
}

export interface PatcherOptions {
  source: string;
  tmUrl?: string;
  inline?: string | null;
  pageVersion?: number;
  storage?: Storage | null;
  /** Discovery endpoint (`/__glot/missing`); none → no discovery. */
  missing?: string;
  /** TM re-poll delays after a report, ms. */
  poll?: number[];
}

export function startPatcher(o: PatcherOptions) {
  let tm: PatcherTm | undefined, skip = "", outs = new Set<string>();
  const dirty = new Set<Element>(), dyn = new WeakSet<Node>(), mine = new WeakMap<object, string>();
  const min = o.pageVersion ?? 0;
  const sent = new Set<string>(), queue: { hash: string; text: string; role: string; url: string }[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined, polling = false;

  const patch = (e: Element) => {
    if (!tm || !e.isConnected || e.closest(MUTED + skip)) return;
    const ctx = template(location.pathname, tm.locale);
    for (const seg of domSegments(e, o.source, skip)) {
      const t = lookup(tm, seg.hash, `${seg.role}|${ctx}`);
      const p = t === undefined ? undefined : check(seg, t);
      if (p) write(seg, p, mine);
      else if (t === undefined) discover(seg);
    }
  };
  // Only text the app rendered on the client (nodes added/changed after boot): the Edge reported the server HTML's.
  const fresh = (n: Node | null) => { for (; n; n = n.parentNode) if (dyn.has(n)) return true; };
  const discover = (seg: DomSegment) => {
    if (!o.missing || !seg.discoverable || seg.el || outs.has(seg.text) || sent.has(seg.hash + seg.role)) return;
    const nodes = seg.gaps.flat();
    if (!nodes.some(fresh) || nodes.some((n) => /\S/.test(n.nodeValue!) && mine.get(n) === n.nodeValue)) return; // not the app's / ours
    if (SESSION_COOKIE.test(document.cookie)) return; // signed in: never discovered (the runtime re-checks HttpOnly ones)
    sent.add(seg.hash + seg.role);
    queue.push({ hash: seg.hash, text: seg.text, role: seg.role, url: location.pathname });
    clearTimeout(timer);
    timer = setTimeout(send, 1500);
  };
  const send = () => {
    const segments = queue.splice(0, 200);
    if (!segments.length || !tm) return;
    fetch(o.missing!, { method: "POST", body: JSON.stringify({ locale: tm.locale, segments }), keepalive: true }).catch(() => {});
    if (!polling && o.tmUrl) {
      polling = true;
      const v = tm.v;
      for (const ms of o.poll ?? [30_000, 120_000, 600_000]) setTimeout(() => tm!.v === v && load(`${o.tmUrl}${o.tmUrl!.includes("?") ? "&" : "?"}since=${v}`).catch(() => {}), ms);
    }
  };
  const flush = () => {
    for (const e of dirty) patch(e);
    dirty.clear();
    mo.takeRecords(); // our own writes
  };
  const mark = (n: Node, d = true) => {
    if (d) dyn.add(n);
    const b = block(n);
    if (b && tm) dirty.add(b);
  };
  const mo = new MutationObserver((recs) => {
    for (const r of recs) {
      if (r.type === "childList") {
        for (const n of r.addedNodes) {
          if (n.nodeType === 1 && pre && n !== pre && !pre.contains(n) && r.target === document.body && !/^(script|style|link)$/.test((n as Element).localName)) { pre.remove(); pre = null; }
          mark(n);
        }
      } else mark(r.target, r.type === "characterData");
    }
    flush();
  });
  let pre = document.querySelector("[data-glot-prerender]");
  mo.observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });

  const use = (x: unknown) => {
    const c = x as PatcherTm | null;
    if (!c || typeof c.t !== "object" || !(c.v >= min) || (tm && tm.v >= c.v)) return false;
    tm = c;
    skip = (c.skip ?? []).filter((s) => { try { return document.querySelector(s), true; } catch { return false; } }).map((s) => `,${s}`).join("");
    outs = new Set(Object.values(c.t).flatMap((v) => (typeof v === "string" ? [v] : Object.values(v))).map(collapseWs));
    // late or new TM: catch up on everything rendered so far
    dirty.add(document.documentElement);
    flush();
    return true;
  };
  const key = `glot-tm:${o.tmUrl}`;
  const load = (url: string) =>
    fetch(url, { credentials: "omit" })
      .then((r) => (r.status === 200 ? r.json() : r.status === 204 ? undefined : Promise.reject(r.status)))
      .then((x?: PatcherTm) => { if (x && use(x)) try { o.storage?.setItem(key, JSON.stringify(x)); } catch { /* quota */ } });
  if (o.inline) { try { use(JSON.parse(o.inline)); } catch { /* ignore a broken inline TM */ } }
  if (!tm && o.tmUrl) {
    let cached: PatcherTm | null = null;
    try { cached = JSON.parse(o.storage?.getItem(key) ?? "null"); } catch { /* storage blocked */ }
    load(o.tmUrl).catch(() => use(cached)); // glot unreachable: last good copy, only if not older than the page
  }
  return { mo, get tm() { return tm; }, flush, send };
}
