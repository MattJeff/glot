// Hydration-safe Edge: translate the strings of the serialized payload that frameworks hydrate from, so the
// client renders the same (translated) text the server HTML shows — no mismatch, no re-render, no flash.
//   Next.js Pages  <script id="__NEXT_DATA__" type="application/json">  (getStaticProps/getServerSideProps props)
//   Next.js App    <script>self.__next_f.push([1,"…"])</script>          (RSC flight: server components' text)
//   Nuxt 3/4       <script type="application/json" id="__NUXT_DATA__">  (devalue array: useFetch/useAsyncData)
//   SvelteKit 2    <script>… kit.start(app, element, { data: [JS literal] })</script>  (load() data, devalue.uneval)
//   SolidStart     <script>self.$R=…;$R[5]={title:"…"}</script>          (seroval: query/createAsync data)
//   React Router 7 / Remix  <script>…streamController.enqueue("[turbo-stream]")</script>  (loader data)
//   Astro islands  <astro-island props="{&quot;title&quot;:[0,&quot;…&quot;]}">  (React/Vue/Svelte island props)
// A string is replaced only when it is exactly (after whitespace folding) the source of a translated segment of
// the page, never a flight reference ("$…"), an object key, or the value of an identifier-like key (id, slug,
// href, type, variant…) — app logic comparing such values keeps working. JSON and JS data literals go through the
// same `scan` (string tokens located in the raw text, edited in place: key order and numbers stay byte-identical).
// Vue pages (Nuxt, Astro islands) get `<html data-allow-mismatch="text">`: Vue still replaces a hard-coded text
// it finds translated (the patcher re-translates it in the same task) but no longer logs a hydration error.
// Text hard-coded in client bundles (client components, Pages-router JSX) is not in these payloads: the Edge
// localizes the served chunks themselves (`edge.ts` bundle() → glot's parser, crates/server/src/edge/bundle.rs).
// What remains (ambiguous texts, no Referer) is re-rendered in the source and the patcher re-applies the TM in
// the same task (MutationObserver → no painted flash), with one recoverable React hydration error.
// ponytail: regex-located scripts + a data-literal scanner; a full HTML tokenizer is the upgrade if pages get exotic.

export type Lookup = (normalizedSource: string) => string | undefined;

const ID_KEYS = /^(id|_id|key|slug|href|url|src|srcset|as|type|kind|variant|status|state|plan|tier|mode|lang|locale|code|name|className|class|style|role|rel|target|method|action|icon|color|size|format|currency|path|route|pathname|query|hash)$/i;
const fold = (s: string) => s.replace(/\s+/g, " ").trim();
/** Same escaping as Next's htmlEscapeJsonString: JSON is safe inside <script>. */
const esc = (json: string) => json.replace(/[<>&\u2028\u2029]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`);

function tr(s: string, lookup: Lookup): string | undefined {
  if (!/\p{L}/u.test(s) || s.startsWith("$")) return undefined;
  const t = lookup(fold(s));
  return t === undefined || t === s ? undefined : t;
}

// ------------------------------------------------------------------ data-literal scanner (= hydration.rs `scan`)

/** A string value of a data literal: `[a, b)` = the literal with its quotes, `s` = decoded. */
export interface Lit { a: number; b: number; s: string; key?: string; top?: number }
interface Frame { k: "{" | "[" | "(" | "m"; key?: string; pend?: string; i: number }
const WS = /\s/;
const isId0 = (c: string) => /[A-Za-z_$]/.test(c);
const isId = (c: string) => /[\w$]/.test(c);

/** JS (and JSON) string literal at `src[i]` → [decoded, end] or undefined (unterminated, raw line break). */
function readStr(src: string, i: number, to: number): [string, number] | undefined {
  const q = src[i];
  let s = "";
  for (let j = i + 1; j < to; ) {
    const c = src[j];
    if (c === q) return [s, j + 1];
    if (c === "\n" || c === "\r") return undefined;
    if (c !== "\\") { s += c; j++; continue; }
    const e = src[j + 1];
    j += 2;
    if (e === undefined) return undefined;
    const hex = (n: number) => { const h = src.slice(j, j + n); if (h.length < n || !/^[0-9a-fA-F]+$/.test(h)) return undefined; j += n; return parseInt(h, 16); };
    let cp: number | undefined;
    switch (e) {
      case "n": s += "\n"; break;
      case "r": s += "\r"; break;
      case "t": s += "\t"; break;
      case "b": s += "\b"; break;
      case "f": s += "\f"; break;
      case "v": s += "\v"; break;
      case "0": if (/[0-9]/.test(src[j] ?? "")) return undefined; s += "\0"; break;
      case "x": if ((cp = hex(2)) === undefined) return undefined; s += String.fromCharCode(cp); break;
      case "u":
        if (src[j] === "{") { const k = src.indexOf("}", j); if (k < 0 || k - j > 7 || !/^[0-9a-fA-F]+$/.test(src.slice(j + 1, k))) return undefined; cp = parseInt(src.slice(j + 1, k), 16); j = k + 1; if (cp > 0x10ffff) return undefined; s += String.fromCodePoint(cp); }
        else { if ((cp = hex(4)) === undefined) return undefined; s += String.fromCharCode(cp); }
        break;
      case "\r": if (src[j] === "\n") j++; break; // line continuation
      case "\n": case " ": case " ": break;
      default: if (/[1-9]/.test(e)) return undefined; s += e; // legacy octal: never in generated data
    }
  }
  return undefined;
}

/**
 * String values of a JSON text or a JS *data literal* (objects, arrays, calls, arrows, identifiers, numbers) in
 * `src[from..to]`. `one`: `src[from]` is a bracket, stop when it closes. Object keys, computed member names
 * (`x["k"]`) and strings outside any bracket are not returned; each value carries the key it belongs to
 * (inherited through arrays: `title: [0, "…"]`) and, directly in a top-level array, its index. undefined =
 * something a generated data literal never contains (template literal, slash = regex/comment, `?`, unbalanced
 * brackets): the caller leaves that script as is.
 */
export function scan(src: string, from = 0, to = src.length, one = false): Lit[] | undefined {
  const out: Lit[] = [];
  const st: Frame[] = [];
  let prev = "";
  const next = (j: number) => { while (j < to && WS.test(src[j])) j++; return src[j]; };
  for (let i = from; i < to; ) {
    const c = src[i];
    const f = st[st.length - 1];
    if (WS.test(c)) { i++; continue; }
    if (c === '"' || c === "'") {
      const r = readStr(src, i, to);
      if (!r) return undefined;
      if (f?.k === "{" && next(r[1]) === ":") f.pend = r[0];
      else if (f && f.k !== "m") {
        out.push({ a: i, b: r[1], s: r[0], key: f.k === "{" ? f.pend : f.k === "[" ? f.key : undefined, top: st.length === 1 && f.k === "[" ? f.i : undefined });
        if (f.k === "{") f.pend = undefined;
      }
      prev = "v";
      i = r[1];
      continue;
    }
    if (isId0(c) || /[0-9]/.test(c)) {
      let j = i + 1;
      while (j < to && (isId(src[j]) || (!isId0(c) && src[j] === "."))) j++;
      if (isId0(c) && f?.k === "{" && next(j) === ":") f.pend = src.slice(i, j);
      prev = "v";
      i = j;
      continue;
    }
    if (c === "`" || c === "/" || c === "?") return undefined;
    if (c === "{" || c === "(") st.push({ k: c, i: 0 });
    else if (c === "[") {
      if (prev === "v" || prev === ")") st.push({ k: "m", i: 0 });
      else {
        st.push({ k: "[", i: 0, key: f?.k === "{" ? f.pend : f?.k === "[" ? f.key : undefined });
        if (f?.k === "{") f.pend = undefined;
      }
    } else if (c === "}" || c === "]" || c === ")") {
      const open = st.pop()?.k;
      if (open !== (c === "}" ? "{" : c === ")" ? "(" : open === "m" ? "m" : "[")) return undefined;
      if (one && !st.length) return out;
      prev = ")";
      i++;
      continue;
    } else if (c === "," && f) {
      f.i++;
      if (f.k === "{") f.pend = undefined;
    }
    prev = c;
    i++;
  }
  return !one && !st.length ? out : undefined;
}

type Edit = [number, number, string];
function splice(s: string, edits: Edit[]): string {
  let o = "", at = 0;
  for (const [a, b, v] of edits) (o += s.slice(at, a) + v), (at = b);
  return o + s.slice(at);
}
/** Edits for the translatable values of `lits`; `skip(l)` = never this one. `enc`: how a value is written. */
function edits(lits: Lit[], lookup: Lookup, skip: (l: Lit) => boolean = () => false, enc = (v: string) => esc(JSON.stringify(v))): Edit[] {
  const e: Edit[] = [];
  for (const l of lits) {
    if ((l.key !== undefined && ID_KEYS.test(l.key)) || skip(l)) continue;
    const t = tr(l.s, lookup);
    if (t !== undefined) e.push([l.a, l.b, enc(t)]);
  }
  return e;
}

/** Each inline `<script>` body (no `src`) of `html` through `f`; `f` returns the new body or undefined. */
function scripts(html: string, f: (body: string) => string | undefined): string {
  return html.replace(/(<script\b[^>]*>)([\s\S]*?)(<\/script>)/g, (all, open: string, body: string, close: string) => {
    if (/\ssrc\s*=/.test(open)) return all;
    const b = f(body);
    return b === undefined ? all : open + b + close;
  });
}

function jsonScript(html: string, id: string, f: (body: string) => string | undefined) {
  const re = new RegExp(`(<script\\b[^>]*\\bid="${id}"[^>]*>)([\\s\\S]*?)(<\\/script>)`);
  return html.replace(re, (all, open: string, body: string, close: string) => { const b = f(body); return b === undefined ? all : open + b + close; });
}

function apply(body: string, e: Edit[] | undefined, n: { c: number }): string | undefined {
  if (!e?.length) return undefined;
  n.c += e.length;
  return splice(body, e);
}

/** Next.js Pages `__NEXT_DATA__`: plain JSON, keys apply. */
const nextData = (html: string, lookup: Lookup, n: { c: number }) =>
  jsonScript(html, "__NEXT_DATA__", (body) => { const l = scan(body); return l && apply(body, edits(l, lookup), n); });

/** Index-addressed flat arrays (devalue, turbo-stream): the top-level entries never translated. */
function flatSkip(a: unknown[], turbo: boolean): Set<number> {
  const skip = new Set<number>();
  for (const x of a) {
    if (x && typeof x === "object" && !Array.isArray(x))
      for (const [k, i] of Object.entries(x)) {
        // devalue: {"id": 7}; turbo-stream: {"_3": 7} where entry 3 is the key "id" (and itself never text)
        const key = turbo ? (/^_\d+$/.test(k) ? a[Number(k.slice(1))] : undefined) : k;
        if (turbo && /^_\d+$/.test(k)) skip.add(Number(k.slice(1)));
        if (typeof key === "string" && ID_KEYS.test(key) && typeof i === "number") skip.add(i);
      }
    // ["Reactive", 3], ["Set", …]: the leading string is a type tag, not text
    if (!turbo && Array.isArray(x) && typeof x[0] === "string") for (const i of x.slice(1)) if (typeof i === "number") skip.add(i);
  }
  return skip;
}
function flat(json: string, lookup: Lookup, turbo: boolean, enc?: (v: string) => string): Edit[] | undefined {
  let a: unknown;
  try { a = JSON.parse(json); } catch { return undefined; }
  if (!Array.isArray(a)) return undefined;
  const l = scan(json);
  if (!l) return undefined;
  const skip = flatSkip(a, turbo);
  return edits(l, lookup, (x) => x.top === undefined || skip.has(x.top), enc);
}

/** Nuxt `__NUXT_DATA__`: devalue's flat array (values are top-level entries, objects/arrays hold indexes). */
const nuxtData = (html: string, lookup: Lookup, n: { c: number }) =>
  jsonScript(html, "__NUXT_DATA__", (body) => apply(body, flat(body, lookup, false), n));

/** SvelteKit: the arguments of `kit.start(app, element, { node_ids, data, … })` (devalue.uneval JS literal). */
const svelteKit = (html: string, lookup: Lookup, n: { c: number }) =>
  scripts(html, (body) => {
    const at = body.indexOf("kit.start(");
    if (at < 0) return undefined;
    const l = scan(body, at + "kit.start".length, body.length, true);
    return l && apply(body, edits(l, lookup), n);
  });

/** SolidStart: seroval's `self.$R[…]` scripts (data in object/array literals and resolver calls). */
const solid = (html: string, lookup: Lookup, n: { c: number }) =>
  scripts(html, (body) => {
    if (!body.includes("$R[")) return undefined;
    const l = scan(body);
    return l && apply(body, edits(l, lookup), n);
  });

/** React Router 7 / Remix: `…streamController.enqueue("<turbo-stream>")`; its first line is the flat array. */
const reactRouter = (html: string, lookup: Lookup, n: { c: number }) =>
  scripts(html, (body) =>
    body.replace(/(streamController\.enqueue\()("(?:[^"\\\n]|\\.)*")(\))/g, (all, pre: string, lit: string, post: string) => {
      let stream: string;
      try { stream = JSON.parse(lit); } catch { return all; }
      const nl = stream.indexOf("\n");
      const line = nl < 0 ? stream : stream.slice(0, nl);
      if (!line.startsWith("[")) return all; // P/E rows (deferred promises): left to the patcher
      const e = flat(line, lookup, true, (v) => JSON.stringify(v));
      if (!e?.length) return all;
      n.c += e.length;
      return pre + esc(JSON.stringify(splice(line, e) + stream.slice(line.length))) + post;
    }),
  );

const ENT: Record<string, string> = { quot: '"', amp: "&", lt: "<", gt: ">", apos: "'" };
/** Attribute value as the browser reads it; undefined for a named entity outside the five basic ones. */
function unattr(v: string): string | undefined {
  let bad = false;
  const s = v.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);/g, (_, e: string) => {
    if (e[0] !== "#") return ENT[e] ?? ((bad = true), "");
    const cp = e[1] === "x" ? parseInt(e.slice(2), 16) : Number(e.slice(1));
    if (!(cp <= 0x10ffff) || (cp >= 0xd800 && cp <= 0xdfff)) return (bad = true), "";
    return String.fromCodePoint(cp);
  });
  return bad ? undefined : s;
}
/** `before` ends with `[0,` (Astro's "plain value" tag), whitespace allowed. */
function plain(before: string): boolean {
  let t = before.trimEnd();
  if (!t.endsWith(",")) return false;
  t = t.slice(0, -1).trimEnd();
  return t.endsWith("0") && t.slice(0, -1).trimEnd().endsWith("[");
}
const reattr = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

/** Astro islands: `props` is JSON of `[type, value]` pairs; only type-0 (plain) strings are text. */
const astro = (html: string, lookup: Lookup, n: { c: number }) =>
  html.replace(/<astro-island\b[^>]*>/g, (tag) =>
    tag.replace(/(\sprops=")([^"]*)(")/, (all, pre: string, raw: string, post: string) => {
      const json = unattr(raw);
      const l = json === undefined ? undefined : scan(json);
      if (!l) return all;
      const e = edits(l, lookup, (x) => !plain(json!.slice(0, x.a)), (v) => JSON.stringify(v));
      if (!e.length) return all;
      n.c += e.length;
      return pre + reattr(splice(json!, e)) + post;
    }),
  );

/** Vue (Nuxt, Astro islands): hard-coded text Vue finds translated is replaced without a console error. */
function vueAllowMismatch(html: string): string {
  if (!/__NUXT_DATA__|id="__nuxt"|<astro-island\b/.test(html)) return html;
  const m = /<html(?=[\s>])[^>]*>/i.exec(html);
  if (!m || /\sdata-allow-mismatch\b/i.test(m[0])) return html;
  const at = m.index + 5;
  return html.slice(0, at) + ' data-allow-mismatch="text"' + html.slice(at);
}

const PUSH = /<script\b[^>]*>\s*self\.__next_f\.push\((\[1,"(?:[^"\\]|\\.)*"\])\)\s*<\/script>/g;
const utf8Len = (s: string) => new TextEncoder().encode(s).length;

/** Flight stream rows: `<hex id>:<json>\n`, or `<hex id>:T<hex byte length>,<raw text>` (length-delimited). */
function flightEdits(stream: string, lookup: Lookup): [number, number, string][] {
  const edits: [number, number, string][] = [];
  let i = 0;
  while (i < stream.length) {
    if (stream[i] === "\n") { i++; continue; }
    const m = /^([0-9a-f]*):/.exec(stream.slice(i, i + 24));
    if (!m) break; // not a row start: stop rather than guess
    let j = i + m[0].length;
    if (stream[j] === "T") {
      const comma = stream.indexOf(",", j);
      if (comma < 0) break;
      let bytes = parseInt(stream.slice(j + 1, comma), 16);
      j = comma + 1;
      while (bytes > 0 && j < stream.length) {
        const cp = stream.codePointAt(j)!;
        bytes -= utf8Len(String.fromCodePoint(cp));
        j += cp > 0xffff ? 2 : 1;
      }
      i = j; // ponytail: T rows (long text) stay untranslated; the patcher covers them
      continue;
    }
    const end = stream.indexOf("\n", j);
    const row = end < 0 ? stream.slice(j) : stream.slice(j, end);
    if (row.startsWith("[") || row.startsWith("{")) {
      const tok = /"((?:[^"\\]|\\.)*)"(\s*:)?/g;
      let key = "";
      for (let t; (t = tok.exec(row)); ) {
        if (t[2]) { key = JSON.parse(`"${t[1]}"`); continue; }
        const s = JSON.parse(`"${t[1]}"`) as string;
        const k = key;
        key = "";
        if (ID_KEYS.test(k)) continue;
        const v = tr(s, lookup);
        if (v !== undefined) edits.push([j + t.index, j + t.index + t[0].length, JSON.stringify(v)]);
      }
    }
    if (end < 0) break;
    i = end + 1;
  }
  return edits;
}

function flight(html: string, lookup: Lookup, n: { c: number }) {
  const pushes = [...html.matchAll(PUSH)];
  if (!pushes.length) return html;
  const chunks = pushes.map((p) => JSON.parse(p[1])[1] as string);
  const starts: number[] = [];
  let off = 0;
  for (const c of chunks) starts.push(off), (off += c.length);
  const edits = flightEdits(chunks.join(""), lookup);
  const out = chunks.slice();
  // apply right to left inside each chunk; an edit straddling two pushes is skipped
  for (const [a, b, v] of edits.reverse()) {
    let k = starts.length - 1;
    while (starts[k] > a) k--;
    if (b > starts[k] + chunks[k].length) continue;
    out[k] = out[k].slice(0, a - starts[k]) + v + out[k].slice(b - starts[k]);
    n.c++;
  }
  let idx = 0;
  return html.replace(PUSH, (all) => {
    const k = idx++;
    return out[k] === chunks[k] ? all : all.replace(pushes[k][1], () => esc(JSON.stringify([1, out[k]])));
  });
}

/** Translate the framework payloads of `html` with the page's segment lookup. `replaced` = strings changed. */
export function translatePayloads(html: string, lookup: Lookup): { html: string; replaced: number } {
  const n = { c: 0 };
  let out = html;
  if (out.includes("__NEXT_DATA__")) out = nextData(out, lookup, n);
  if (out.includes("__NUXT_DATA__")) out = nuxtData(out, lookup, n);
  if (out.includes("self.__next_f.push")) out = flight(out, lookup, n);
  if (out.includes("kit.start(")) out = svelteKit(out, lookup, n);
  if (out.includes("$R[")) out = solid(out, lookup, n);
  if (out.includes("streamController.enqueue(")) out = reactRouter(out, lookup, n);
  if (out.includes("<astro-island")) out = astro(out, lookup, n);
  return { html: vueAllowMismatch(out), replaced: n.c };
}

/** Largest TM inlined in the page (bytes of JSON); a larger one is fetched by the patcher (`data-tm`). */
export const TM_INLINE_MAX = 32_768;
/** `<script type="application/json" id="glot-tm">` carrying the page's TM when it is at most TM_INLINE_MAX bytes:
 * the patcher translates text the app renders right after load (after-mount, client-only) without waiting for a
 * request, so a cold visit paints no source text. "" when larger (hydration.rs `tm_inline`). */
export const tmInline = (json: string) =>
  new TextEncoder().encode(json).length <= TM_INLINE_MAX ? `<script type="application/json" id="glot-tm">${esc(json)}</script>` : "";

const attr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
/** `<script async>` of the client patcher (glot_core::edge::hydration::patcher_tag); `missing` = discovery endpoint. */
export const patcherTag = (scriptUrl: string, tmUrl: string, sourceLocale: string, missing?: string) =>
  `<script async src="${attr(scriptUrl)}" data-tm="${attr(tmUrl)}" data-source="${attr(sourceLocale)}"${missing ? ` data-missing="${attr(missing)}"` : ""}></script>`;
/** Insert `tag` after `<meta name="glot-tm" …>`, else right after `<head>` (glot_core::edge::hydration::inject_head). */
export function injectHead(html: string, tag: string): string {
  const m = /<meta name="glot-tm"[^>]*>/i.exec(html) ?? /<head(?:\s[^>]*)?>/i.exec(html);
  return m ? html.slice(0, m.index + m[0].length) + tag + html.slice(m.index + m[0].length) : html;
}
