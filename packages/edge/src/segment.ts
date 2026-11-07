// glot Edge segmentation + protection: the exact mirror of crates/core/src/edge/{segment,protect}.rs.
// Rendered HTML → segments (block text runs with inline placeholders `<0>…</0>` / `<1/>`, values `{0}`,
// translatable attributes), and translateHtml() to put translations back. Shared fixtures:
// crates/core/tests/fixtures/edge/*.html → *.segments.json (test/segment.test.ts). Change both sides together.

export type ValueKind =
  | "number" | "url" | "code" | "public"
  | "email" | "phone" | "card" | "iban" | "uuid" | "token" | "long_number";
export interface Value { kind: ValueKind; value: string }
export type Placeholder =
  | { pair: true; name: string; open: [number, number]; close: [number, number] }
  | { pair: false; name: string; range: [number, number] };
export interface Segment {
  hash: string; // filled by segmentHashed()/translateHtml(); "" from segment()
  text: string;
  role: string;
  key?: string;
  discoverable: boolean;
  values: Value[];
  tags: Placeholder[];
  span: [number, number];
  attr?: string;
}
export interface SegmentOpts { sourceLocale?: string; skip?: string[]; dnt?: string[]; piiAllow?: string[] }

export const isPii = (k: ValueKind) => !["number", "url", "code", "public"].includes(k);

// ------------------------------------------------------------------------------------------- protect

const HTML_WS = " \t\n\r\f";
const isWs = (c: string) => c !== "" && HTML_WS.includes(c);
const isSep = (c: string) => isWs(c) || c === " " || c === " ";
export const allWs = (s: string) => [...s].every(isWs);
const ALPHA = /\p{Alphabetic}/u;
const ALNUM = /[\p{Alphabetic}\p{N}]/u;
const asciiLower = (s: string) => s.replace(/[A-Z]+/g, (m) => m.toLowerCase());
const eqIgnoreAscii = (a: string, b: string) => asciiLower(a) === asciiLower(b);
const isDigit = (c: string | undefined) => c !== undefined && c >= "0" && c <= "9";
const isAsciiAlpha = (c: string | undefined) => c !== undefined && /^[A-Za-z]$/.test(c);
const isAsciiAlnum = (c: string | undefined) => isDigit(c) || isAsciiAlpha(c);

export function collapseWs(s: string): string {
  let out = "";
  let ws = false;
  for (const c of s) {
    if (isWs(c)) ws = true;
    else {
      if (ws && out) out += " ";
      ws = false;
      out += c;
    }
  }
  return out;
}

const NAMED: Record<string, string> = {
  amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ", copy: "©", reg: "®", trade: "™",
  hellip: "…", mdash: "—", ndash: "–", laquo: "«", raquo: "»", lsquo: "‘", rsquo: "’", ldquo: "“",
  rdquo: "”", euro: "€", middot: "·", bull: "•", times: "×",
};

export function decodeEntities(s: string): string {
  if (!s.includes("&")) return s;
  return s.replace(/&([A-Za-z0-9#]{1,10});/g, (m, name: string) => {
    let code: number | undefined;
    if (/^#[xX][0-9a-fA-F]{1,6}$/.test(name)) code = parseInt(name.slice(2), 16);
    else if (/^#[0-9]{1,7}$/.test(name)) code = parseInt(name.slice(1), 10);
    else if (name.startsWith("#")) return m;
    else return Object.hasOwn(NAMED, name) ? NAMED[name] : m;
    return code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) ? m : String.fromCodePoint(code);
  });
}

export const escapeText = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
export const escapeAttr = (s: string) => escapeText(s).replace(/"/g, "&quot;");
export const hasLetter = (s: string) => ALPHA.test(s);

const isEmail = (w: string) => {
  const at = w.indexOf("@");
  if (at < 0) return false;
  const local = w.slice(0, at), domain = w.slice(at + 1);
  const tld = domain.split(".").pop() ?? "";
  return local !== "" && /^[A-Za-z0-9._%+-]+$/.test(local) && domain.includes(".") &&
    /^[A-Za-z0-9.-]+$/.test(domain) && tld.length >= 2 && /^[A-Za-z]+$/.test(tld);
};
const isUuid = (w: string) => /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/.test(w);
const isIban = (w: string) => /^[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}$/.test(w);
const isToken = (w: string) =>
  /[0-9]/.test(w) && /[A-Za-z]/.test(w) && /^[A-Za-z0-9_\-.=+/]+$/.test(w) &&
  (w.length >= 20 || (w.length >= 16 && /^[0-9a-fA-F]+$/.test(w)));
const isCode = (w: string) => w.endsWith("()") || w.includes("::") || /[A-Za-z0-9]_[A-Za-z0-9]/.test(w);
const isUrl = (w: string) => {
  const l = asciiLower(w);
  return l.startsWith("http://") || l.startsWith("https://") || l.startsWith("www.") || l.includes("://");
};
function wordKind(w: string): ValueKind | undefined {
  if (isUrl(w)) return "url";
  if (isEmail(w)) return "email";
  if (isUuid(w)) return "uuid";
  if (isIban(w)) return "iban";
  if (isToken(w)) return "token";
  if (isCode(w)) return "code";
}

function luhn(d: string): boolean {
  let sum = 0;
  for (let i = 0; i < d.length; i++) {
    let x = +d[d.length - 1 - i];
    if (i % 2 === 1) { x *= 2; if (x > 9) x -= 9; }
    sum += x;
  }
  return sum % 10 === 0;
}
const NUM_SEPS = " .-/,:  ";
const groupedByThousands = (run: string) => {
  const g = run.split(/[ .\-/,:\u00a0\u202f]/);
  return g.length > 1 && g[0].length >= 1 && g[0].length <= 3 && g.slice(1).every((x) => x.length === 3);
};
function numberKind(run: string): ValueKind {
  const digits = run.replace(/[^0-9]/g, "");
  const n = digits.length;
  if (n >= 13 && n <= 19 && luhn(digits)) return "card";
  if (n >= 9 && !groupedByThousands(run.replace(/^\++/, ""))) return run.startsWith("+") || run.startsWith("0") ? "phone" : "long_number";
  return "number";
}

const STREET = ["rue", "avenue", "av.", "boulevard", "bd", "chemin", "allée", "impasse", "quai", "street", "road", "lane",
  "drive", "calle", "avenida", "via", "strasse", "straße", "straat", "rua"];
const splitSep = (t: string) => t.split(/[ \t\n\r\f  ]/).filter((w) => w !== "");

export function looksLikeAddress(text: string): boolean {
  const words = splitSep(text).map((w) => w.replace(/,+$/, "").toLowerCase());
  const house = (w: string) => {
    const num = w.replace(/[A-Za-z]+$/, "");
    return num !== "" && num.length <= 4 && w.length - num.length <= 3 && /^[0-9]+$/.test(num);
  };
  return words.some((w, j) => STREET.includes(w) && words.slice(Math.max(0, j - 3), j).some(house));
}

const LEAD_PUNCT = "([{«\"'“‘";
const TRAIL_PUNCT = ".,;:!?)]}»\"'”’";
const prevChar = (s: string, i: number) => (i <= 0 ? "" : String.fromCodePoint(s.codePointAt(i >= 2 && /[\udc00-\udfff]/.test(s[i - 1]) && /[\ud800-\udbff]/.test(s[i - 2]) ? i - 2 : i - 1)!));
const nextChar = (s: string, i: number) => (i >= s.length ? "" : String.fromCodePoint(s.codePointAt(i)!));

/** Replace values in `text` with `{n}` (n continues from values.length). */
export function mask(text: string, values: Value[], allow: string[]): string {
  const hits: [number, number, ValueKind][] = [];
  let pos = 0;
  while (pos < text.length) {
    let start = pos;
    while (start < text.length && isSep(text[start])) start++;
    if (start >= text.length) break;
    let end = start;
    while (end < text.length && !isSep(text[end])) end++;
    let cs = start;
    while (cs < end && LEAD_PUNCT.includes(text[cs])) cs++;
    let ce = end;
    while (ce > cs && TRAIL_PUNCT.includes(text[ce - 1])) ce--;
    if (text.slice(cs, ce).includes("(") && text[ce] === ")" && ce < end) ce++;
    if (cs < ce) {
      const k = wordKind(text.slice(cs, ce));
      if (k) hits.push([cs, ce, k]);
    }
    pos = end;
  }
  const runs: [number, number, ValueKind][] = [];
  let p = 0;
  while (p < text.length) {
    if (!isDigit(text[p])) { p++; continue; }
    let start = p;
    if (start > 0 && text[start - 1] === "+" && (start === 1 || isSep(text[start - 2]))) start--;
    let end = p + 1;
    for (;;) {
      if (isDigit(text[end])) { end++; continue; }
      if (end < text.length && NUM_SEPS.includes(text[end]) && isDigit(text[end + 1])) { end += 2; continue; }
      break;
    }
    const before = prevChar(text, start);
    const after = nextChar(text, end);
    const beforeOk = !(before && (ALNUM.test(before) || before === "_"));
    const afterOk = !(after && (ALPHA.test(after) || after === "_"));
    if (beforeOk && afterOk && !hits.some((h) => h[0] < end && start < h[1])) runs.push([start, end, numberKind(text.slice(start, end))]);
    p = end;
  }
  hits.push(...runs);
  hits.sort((a, b) => a[0] - b[0]);
  let out = "";
  let last = 0;
  for (const [s, e, k0] of hits) {
    const v = text.slice(s, e);
    const kind = isPii(k0) && allow.some((a) => eqIgnoreAscii(a, v)) ? "public" : k0;
    out += text.slice(last, s) + `{${values.length}}`;
    values.push({ kind, value: v });
    last = e;
  }
  return out + text.slice(last);
}

type Attrs = [string, string][];
const get = (attrs: Attrs, n: string) => attrs.find(([k]) => k === n)?.[1];
const classes = (v: string | undefined) => (v ?? "").split(/[ \t\n\r\f]+/).filter(Boolean);

interface Selector { tag?: string; id?: string; classes: string[]; attrs: [string, string | undefined][] }
export function parseSelector(s0: string): Selector | undefined {
  const s = s0.trim();
  if (!s || /[ >+~,]/.test(s)) return;
  const sel: Selector = { classes: [], attrs: [] };
  const identEnd = (r: string) => { const i = r.search(/[.#[]/); return i < 0 ? r.length : i; };
  let rest = s;
  const n = identEnd(rest);
  if (n > 0) { const t = rest.slice(0, n); if (t !== "*") sel.tag = asciiLower(t); rest = rest.slice(n); }
  while (rest) {
    const c = rest[0];
    rest = rest.slice(1);
    if (c === "." || c === "#") {
      const n = identEnd(rest);
      if (!n) return;
      if (c === ".") sel.classes.push(rest.slice(0, n)); else sel.id = rest.slice(0, n);
      rest = rest.slice(n);
    } else if (c === "[") {
      const close = rest.indexOf("]");
      if (close < 0) return;
      const inner = rest.slice(0, close);
      rest = rest.slice(close + 1);
      const eq = inner.indexOf("=");
      if (eq >= 0) sel.attrs.push([asciiLower(inner.slice(0, eq).trim()), inner.slice(eq + 1).trim().replace(/^["']+|["']+$/g, "")]);
      else sel.attrs.push([asciiLower(inner.trim()), undefined]);
    } else return;
  }
  return sel;
}
const selMatches = (s: Selector, tag: string, attrs: Attrs) =>
  (s.tag === undefined || s.tag === tag) && (s.id === undefined || get(attrs, "id") === s.id) &&
  s.classes.every((c) => classes(get(attrs, "class")).includes(c)) &&
  s.attrs.every(([k, v]) => { const a = get(attrs, k); return a !== undefined && (v === undefined || a === v); });

type Skip = "code" | "form" | "opaque";
export const CODE_TAGS = ["script", "style", "pre", "code", "kbd", "samp", "var", "svg", "math", "xmp", "plaintext"];
function skipOf(tag: string, attrs: Attrs, sels: Selector[]): Skip | undefined {
  const ce = get(attrs, "contenteditable");
  if (eqIgnoreAscii(get(attrs, "translate") ?? "", "no") || classes(get(attrs, "class")).includes("notranslate") ||
    get(attrs, "data-glot-skip") !== undefined || get(attrs, "data-glot-user") !== undefined ||
    (ce !== undefined && !eqIgnoreAscii(ce, "false")) || sels.some((s) => selMatches(s, tag, attrs))) return "opaque";
  if (CODE_TAGS.includes(tag)) return "code";
  if (tag === "textarea" || tag === "select") return "form";
}

// ------------------------------------------------------------------------------------------- tokenizer

export interface Attr { name: string; value: string; range: [number, number] }
export type Tok =
  | { t: "start"; name: string; attrs: Attr[]; selfClosing: boolean; range: [number, number] }
  | { t: "end"; name: string; range: [number, number] }
  | { t: "text"; range: [number, number] }
  | { t: "comment"; range: [number, number] };

const RAW_TEXT = ["script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes"];
const asciiWs = (c: string | undefined) => c !== undefined && isWs(c);

export function tokenize(s: string): Tok[] {
  const n = s.length;
  const lower = asciiLower(s);
  const toks: Tok[] = [];
  let textStart = 0;
  let i = 0;
  const find = (from: number, pat: string) => { const j = s.indexOf(pat, from); return j < 0 ? undefined : j; };
  const nameEnd = (from: number) => { let j = from; while (j < n && !asciiWs(s[j]) && s[j] !== "/" && s[j] !== ">") j++; return j; };
  const flush = () => { if (textStart < i) toks.push({ t: "text", range: [textStart, i] }); };
  while (i < n) {
    if (s[i] !== "<" || i + 1 >= n) { i++; continue; }
    const c = s[i + 1];
    if (s.startsWith("<!--", i)) {
      flush();
      const j = find(i + 4, "-->");
      const end = j === undefined ? n : j + 3;
      toks.push({ t: "comment", range: [i, end] });
      i = textStart = end;
    } else if (c === "!" || c === "?") {
      flush();
      const j = find(i, ">");
      const end = j === undefined ? n : j + 1;
      toks.push({ t: "comment", range: [i, end] });
      i = textStart = end;
    } else if (c === "/" && i + 2 < n && isAsciiAlpha(s[i + 2])) {
      flush();
      const ne = nameEnd(i + 2);
      const j = find(ne, ">");
      const end = j === undefined ? n : j + 1;
      toks.push({ t: "end", name: lower.slice(i + 2, ne), range: [i, end] });
      i = textStart = end;
    } else if (isAsciiAlpha(c)) {
      flush();
      const ne = nameEnd(i + 1);
      const name = lower.slice(i + 1, ne);
      let j = ne;
      const attrs: Attr[] = [];
      let selfClosing = false;
      for (;;) {
        while (j < n && (asciiWs(s[j]) || s[j] === "/")) { selfClosing = s[j] === "/"; j++; }
        if (j >= n || s[j] === ">") break;
        selfClosing = false;
        const ns = j;
        j++;
        while (j < n && !asciiWs(s[j]) && s[j] !== "=" && s[j] !== ">" && s[j] !== "/") j++;
        const aname = lower.slice(ns, j);
        let k = j;
        while (k < n && asciiWs(s[k])) k++;
        let value = "";
        if (k < n && s[k] === "=") {
          k++;
          while (k < n && asciiWs(s[k])) k++;
          if (k < n && (s[k] === '"' || s[k] === "'")) {
            const q = s[k];
            const vs = k + 1;
            let ve = s.indexOf(q, vs);
            if (ve < 0) ve = n;
            value = s.slice(vs, ve);
            j = Math.min(ve + 1, n);
          } else {
            const vs = k;
            while (k < n && !asciiWs(s[k]) && s[k] !== ">") k++;
            value = s.slice(vs, k);
            j = k;
          }
        }
        attrs.push({ name: aname, value: decodeEntities(value), range: [ns, j] });
      }
      const end = Math.min(j + 1, n);
      toks.push({ t: "start", name, attrs, selfClosing, range: [i, end] });
      i = textStart = end;
      if (RAW_TEXT.includes(name) && !selfClosing) {
        const pat = `</${name}`;
        let from = i;
        let close: number;
        for (;;) {
          const x = lower.indexOf(pat, from);
          if (x < 0) { close = n; break; }
          const after = x + pat.length;
          if (after >= n || asciiWs(s[after]) || s[after] === "/" || s[after] === ">") { close = x; break; }
          from = after;
        }
        if (i < close) toks.push({ t: "text", range: [i, close] });
        i = textStart = close;
      }
    } else i++;
  }
  flush();
  return toks;
}

// ------------------------------------------------------------------------------------------- segmenter

export const VOID = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
export const INLINE = ["a", "abbr", "b", "bdi", "bdo", "big", "br", "button", "cite", "code", "data", "del", "dfn", "em", "font", "i",
  "img", "input", "ins", "kbd", "label", "link", "mark", "meta", "meter", "output", "picture", "progress", "q", "ruby", "rp",
  "rt", "s", "samp", "select", "small", "span", "strike", "strong", "sub", "sup", "textarea", "time", "tt", "u", "var", "wbr",
  "svg", "math", "video", "audio", "canvas", "iframe", "object", "embed"];
export const MEDIA = ["img", "picture", "video", "audio", "canvas", "iframe", "object", "embed"];
export const ROLE_INLINE = ["a", "button", "label"];
export const META_NAMES = ["description", "og:title", "og:description", "twitter:title", "twitter:description"];
const P_CLOSERS = ["address", "article", "aside", "blockquote", "details", "dialog", "div", "dl", "dd", "dt", "fieldset",
  "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "li", "main", "menu",
  "nav", "ol", "p", "pre", "section", "table", "ul"];

const NORMAL = 0, OPAQUE = 1, SKIP = 2;
type Kind =
  | { k: "block"; role: string; key?: string; hasChildBlock: boolean }
  | { k: "inline"; id: number }
  | { k: "voidEl"; item: number }
  | { k: "muted" };
interface Frame { name: string; level: number; kind: Kind }
type Item =
  | { t: "text"; range: [number, number]; text: string }
  | { t: "open"; id: number; name: string; role?: string; key?: string; range: [number, number] }
  | { t: "close"; id: number; range: [number, number] }
  | { t: "void"; name: string; range: [number, number] }
  | { t: "comment"; range: [number, number] };
const trimmable = (it: Item) => (it.t === "text" ? allWs(it.text) : it.t === "void" || it.t === "comment");

const roleOf = (tag: string, attrs: Attrs) => { const r = asciiLower((get(attrs, "role") ?? "").trim()); return r || tag; };
const keyOf = (attrs: Attrs) => get(attrs, "data-glot-key") ?? get(attrs, "data-glot");
function impliedClose(open: string, n: string): boolean {
  switch (open) {
    case "p": return P_CLOSERS.includes(n);
    case "li": return n === "li";
    case "dt": case "dd": return n === "dt" || n === "dd";
    case "td": case "th": return ["td", "th", "tr", "tbody", "thead", "tfoot"].includes(n);
    case "tr": return ["tr", "tbody", "thead", "tfoot"].includes(n);
    case "thead": case "tbody": case "tfoot": return ["tbody", "thead", "tfoot"].includes(n);
  }
  return false;
}

/** Text without `<k>`, `</k>`, `<k/>`, `{k}`. */
export type Part = { p: "text"; s: string } | { p: "open" | "close" | "void" | "val"; n: number };
export function parseTemplate(t: string, ntags: number, nvalues: number): Part[] {
  const out: Part[] = [];
  const re = /<(\/?)(\d+)(\/?)>|\{(\d+)\}/g;
  let last = 0;
  for (let m; (m = re.exec(t)); ) {
    let part: Part | undefined;
    if (m[4] !== undefined) { const n = +m[4]; if (n < nvalues) part = { p: "val", n }; }
    else if (!(m[1] && m[3])) {
      const n = +m[2];
      if (n < ntags) part = { p: m[1] ? "close" : m[3] ? "void" : "open", n };
    }
    if (!part) { re.lastIndex = m.index + 1; continue; }
    if (last < m.index) out.push({ p: "text", s: t.slice(last, m.index) });
    out.push(part);
    last = re.lastIndex;
  }
  if (last < t.length) out.push({ p: "text", s: t.slice(last) });
  return out;
}
export const stripPlaceholders = (t: string) =>
  parseTemplate(t, Infinity, Infinity).filter((p): p is { p: "text"; s: string } => p.p === "text").map((p) => p.s).join(" ");

class Segmenter {
  stack: Frame[] = [];
  run: Item[] = [];
  nextId = 0;
  out: Segment[] = [];
  opts: Required<Omit<SegmentOpts, "skip">>;
  sels: Selector[];
  constructor(opts: Required<Omit<SegmentOpts, "skip">>, sels: Selector[]) { this.opts = opts; this.sels = sels; }

  level() { return this.stack.length ? this.stack[this.stack.length - 1].level : NORMAL; }

  start(name: string, attrs: Attr[], selfClosing: boolean, range: [number, number]) {
    const kv: Attrs = attrs.map((a) => [a.name, a.value]);
    if (this.level() === NORMAL) {
      for (;;) {
        let i = -1;
        for (let j = this.stack.length - 1; j >= 0; j--) if (this.stack[j].kind.k === "block") { i = j; break; }
        if (i < 0) break;
        const onlyInline = this.stack.slice(i + 1).every((f) => f.kind.k === "inline");
        if (onlyInline && impliedClose(this.stack[i].name, name)) this.closeTo(i, [range[0], range[0]]);
        else break;
      }
    }
    const cur = this.level();
    const skip = cur === SKIP ? undefined : skipOf(name, kv, this.sels);
    if (cur !== SKIP && skip !== "code" && skip !== "opaque") this.attrSegments(name, attrs, kv);
    const isVoid = VOID.includes(name) || (selfClosing && (cur === SKIP || name === "svg" || name === "math"));
    const own = skip ? SKIP : MEDIA.includes(name) ? OPAQUE : NORMAL;
    const level = Math.max(cur, own);
    const inline = INLINE.includes(name);
    let kind: Kind;
    if (cur !== NORMAL) kind = { k: "muted" };
    else if (level === NORMAL && inline) {
      if (isVoid) { this.run.push({ t: "void", name, range }); return; }
      const id = this.nextId++;
      this.run.push({ t: "open", id, name, role: ROLE_INLINE.includes(name) || get(kv, "role") !== undefined ? roleOf(name, kv) : undefined, key: keyOf(kv), range });
      kind = { k: "inline", id };
    } else if (inline) {
      this.run.push({ t: "void", name, range });
      kind = { k: "voidEl", item: this.run.length - 1 };
    } else {
      for (let j = this.stack.length - 1; j >= 0; j--) {
        const k = this.stack[j].kind;
        if (k.k === "block") { k.hasChildBlock = true; break; }
      }
      this.flush();
      kind = level === NORMAL ? { k: "block", role: roleOf(name, kv), key: keyOf(kv), hasChildBlock: false } : { k: "muted" };
    }
    if (!isVoid) this.stack.push({ name, level, kind });
  }

  end(name: string, range: [number, number]) {
    for (let i = this.stack.length - 1; i >= 0; i--) if (this.stack[i].name === name) return this.closeTo(i, range);
  }

  closeTo(i: number, range: [number, number]) {
    while (this.stack.length > i) {
      const r: [number, number] = this.stack.length === i + 1 ? range : [range[0], range[0]];
      const k = this.stack[this.stack.length - 1].kind;
      if (k.k === "inline") this.run.push({ t: "close", id: k.id, range: r });
      else if (k.k === "voidEl") { const it = this.run[k.item]; if (it?.t === "void") it.range = [it.range[0], r[1]]; }
      else if (k.k === "block") this.flush();
      this.stack.pop();
    }
  }

  attrSegments(tag: string, attrs: Attr[], kv: Attrs) {
    for (const a of attrs) {
      let role: string;
      if (["title", "alt", "aria-label", "aria-description", "placeholder"].includes(a.name)) role = `${tag}:${a.name}`;
      else if (a.name === "value" && tag === "input" && ["button", "submit", "reset"].includes(asciiLower(get(kv, "type") ?? "-"))) role = "input:value";
      else if (a.name === "content" && tag === "meta") {
        const n = get(kv, "name") ?? get(kv, "property");
        if (n === undefined || !META_NAMES.includes(asciiLower(n))) continue;
        role = `meta:${asciiLower(n)}`;
      } else continue;
      const values: Value[] = [];
      const text = collapseWs(mask(a.value, values, this.opts.piiAllow));
      const seg = this.make(text, values, a.value, role, undefined);
      if (seg) { seg.span = a.range; seg.attr = a.name; this.out.push(seg); }
    }
  }

  make(text: string, values: Value[], raw: string, role: string, key: string | undefined): Segment | undefined {
    let plain = stripPlaceholders(text);
    for (const t of this.opts.dnt) if (t) plain = plain.split(t).join(" ");
    if (!hasLetter(plain)) return;
    const discoverable = !values.some((v) => isPii(v.kind)) && !looksLikeAddress(raw);
    const seg: Segment = { hash: "", text, role, discoverable, values, tags: [], span: [0, 0] };
    if (key !== undefined) seg.key = key;
    return seg;
  }

  carrier(): [string, string | undefined] {
    for (let j = this.stack.length - 1; j >= 0; j--) {
      const k = this.stack[j].kind;
      if (k.k === "block") return [k.role, k.hasChildBlock ? undefined : k.key];
    }
    return ["body", undefined];
  }

  flush() {
    if (!this.run.length) return;
    // Empty inline elements (`<i class="icon"></i>`) are voids: trimmed at edges, `<k/>` inside.
    const items: Item[] = [];
    for (const it of this.run) {
      if (it.t === "close") {
        const last = items[items.length - 1];
        const at = items.length - (last?.t === "text" && allWs(last.text) ? 1 : 0);
        const o = items[at - 1];
        if (o?.t === "open" && o.id === it.id) {
          items.length = at - 1;
          items.push({ t: "void", name: o.name, range: [o.range[0], it.range[1]] });
          continue;
        }
      }
      items.push(it);
    }
    this.run = [];
    const [role, key] = this.carrier();
    const opened = new Set<number>(), matched = new Set<number>();
    for (const it of items) {
      if (it.t === "open") opened.add(it.id);
      else if (it.t === "close" && opened.has(it.id)) matched.add(it.id);
    }
    let start = 0;
    items.forEach((it, i) => {
      if ((it.t === "open" || it.t === "close") && !matched.has(it.id)) {
        this.piece(items.slice(start, i), role, key);
        start = i + 1;
      }
    });
    this.piece(items.slice(start), role, key);
  }

  piece(s: Item[], role: string, key: string | undefined) {
    for (;;) {
      while (s.length && trimmable(s[0])) s = s.slice(1);
      while (s.length && trimmable(s[s.length - 1])) s = s.slice(0, -1);
      const f = s[0], l = s[s.length - 1];
      if (s.length >= 2 && f.t === "open" && l.t === "close" && f.id === l.id) {
        if (f.role !== undefined) role = f.role;
        if (f.key !== undefined) key = f.key;
        s = s.slice(1, -1);
      } else break;
    }
    if (!s.length) return;
    let depth = 0, loose = false, spaced = false, allRoles = true, gstart = 0;
    const groups: [number, number][] = [];
    s.forEach((it, i) => {
      if (it.t === "open") { if (depth === 0) { gstart = i; allRoles &&= it.role !== undefined; } depth++; }
      else if (it.t === "close") { depth = Math.max(0, depth - 1); if (depth === 0) groups.push([gstart, i + 1]); }
      else if (it.t === "text" && depth === 0) { const ws = allWs(it.text); loose ||= !ws; spaced ||= ws; }
    });
    if (!loose && (allRoles || !spaced)) {
      for (const [a, b] of groups) this.piece(s.slice(a, b), role, undefined);
      return;
    }
    this.emit(s, role, key);
  }

  emit(s: Item[], role: string, key: string | undefined) {
    let text = "", raw = "";
    const values: Value[] = [];
    const tags: Placeholder[] = [];
    const ids = new Map<number, number>();
    for (const it of s) {
      if (it.t === "text") { raw += it.text; text += mask(it.text, values, this.opts.piiAllow); }
      else if (it.t === "open") { ids.set(it.id, tags.length); text += `<${tags.length}>`; tags.push({ pair: true, name: it.name, open: it.range, close: [0, 0] }); }
      else if (it.t === "close") { const k = ids.get(it.id)!; (tags[k] as { close: [number, number] }).close = it.range; text += `</${k}>`; }
      else if (it.t === "void") { text += `<${tags.length}/>`; tags.push({ pair: false, name: it.name, range: it.range }); }
      raw += " ";
    }
    const seg = this.make(collapseWs(text), values, raw, role, key);
    if (seg) { seg.tags = tags; seg.span = [s[0].range[0], s[s.length - 1].range[1]]; this.out.push(seg); }
  }
}

const fullOpts = (o: SegmentOpts) => ({ sourceLocale: o.sourceLocale ?? "en", dnt: o.dnt ?? [], piiAllow: o.piiAllow ?? [] });

/** All segments of `html` in document order; `hash` is "" (see segmentHashed). */
export function segment(html: string, opts: SegmentOpts = {}): Segment[] {
  const sels = (opts.skip ?? []).map(parseSelector).filter((s): s is Selector => !!s);
  const sg = new Segmenter(fullOpts(opts), sels);
  for (const t of tokenize(html)) {
    if (t.t === "start") sg.start(t.name, t.attrs, t.selfClosing, t.range);
    else if (t.t === "end") sg.end(t.name, t.range);
    else if (sg.level() === NORMAL) {
      if (t.t === "text") sg.run.push({ t: "text", range: t.range, text: decodeEntities(html.slice(t.range[0], t.range[1])) });
      else sg.run.push({ t: "comment", range: t.range });
    }
  }
  sg.closeTo(0, [html.length, html.length]);
  sg.flush();
  return sg.out.sort((a, b) => a.span[0] - b.span[0]);
}

/** `sha256(source_locale + "\0" + text)`, first 16 bytes hex: the TM key (glot_core::edge::tm::text_hash). */
export const segmentKey = (sourceLocale: string, text: string) => hash16(`${sourceLocale}\u0000${text}`);

export function segmentHashed(html: string, opts: SegmentOpts = {}): Segment[] {
  const segs = segment(html, opts);
  for (const s of segs) s.hash = segmentKey(opts.sourceLocale ?? "en", s.text);
  return segs;
}

const K = new Uint32Array(64);
const isPrime = (n: number) => { for (let d = 2; d * d <= n; d++) if (n % d === 0) return false; return true; };
for (let n = 2, i = 0; i < 64; n++) if (isPrime(n)) K[i++] = (Math.cbrt(n) % 1) * 2 ** 32;
const H0 = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
/** SHA-256 of `s` (UTF-8), first 16 bytes in hex, synchronously (the browser patcher can't await in a MutationObserver). */
export function hash16(s: string): string {
  const m = new TextEncoder().encode(s), l = m.length, n = ((l + 9 + 63) >> 6) << 6, b = new Uint8Array(n);
  b.set(m);
  b[l] = 0x80;
  new DataView(b.buffer).setUint32(n - 4, l * 8);
  const h = H0.slice(), w = new Uint32Array(64), r = (x: number, k: number) => (x >>> k) | (x << (32 - k));
  for (let o = 0; o < n; o += 64) {
    for (let i = 0; i < 16; i++) w[i] = (b[o + 4 * i] << 24) | (b[o + 4 * i + 1] << 16) | (b[o + 4 * i + 2] << 8) | b[o + 4 * i + 3];
    for (let i = 16; i < 64; i++) w[i] = w[i - 16] + (r(w[i - 15], 7) ^ r(w[i - 15], 18) ^ (w[i - 15] >>> 3)) + w[i - 7] + (r(w[i - 2], 17) ^ r(w[i - 2], 19) ^ (w[i - 2] >>> 10));
    let [a, bb, c, d, e, f, g, hh] = h;
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (r(e, 6) ^ r(e, 11) ^ r(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) | 0;
      const t2 = ((r(a, 2) ^ r(a, 13) ^ r(a, 22)) + ((a & bb) ^ (a & c) ^ (bb & c))) | 0;
      hh = g; g = f; f = e; e = (d + t1) | 0; d = c; c = bb; bb = a; a = (t1 + t2) | 0;
    }
    [a, bb, c, d, e, f, g, hh].forEach((x, i) => (h[i] = (h[i] + x) | 0));
  }
  return h.slice(0, 4).map((x) => (x >>> 0).toString(16).padStart(8, "0")).join("");
}

/** Page template (glot_core::edge::tm::template): locale prefix dropped, ids → `:id`, slug leaf → `*`. */
export function template(path: string, locale: string): string {
  const p = path.split("/").filter(Boolean);
  if (p[0]?.toLowerCase() === locale.toLowerCase()) p.shift();
  return "/" + p.map((s, i) => {
    const digits = s.replace(/\D/g, "").length;
    if (digits === s.length || (s.length >= 8 && /^[0-9a-f-]+$/i.test(s) && digits > 0) || s.length >= 24) return ":id";
    return p.length >= 2 && i === p.length - 1 && /[-_]/.test(s) ? "*" : s.toLowerCase();
  }).join("/");
}

// ------------------------------------------------------------------------------------------- apply

/** Translation template → parts, or undefined unless every placeholder of `seg` appears exactly once, nested right. */
export function check(seg: { tags: readonly { pair: boolean }[]; values: readonly unknown[]; attr?: unknown }, tr: string): Part[] | undefined {
  const parts = parseTemplate(tr, Infinity, Infinity);
  const seenTag = new Array<number>(seg.tags.length).fill(0), seenVal = new Array<number>(seg.values.length).fill(0);
  const stack: number[] = [];
  for (const p of parts) {
    if (p.p === "text") continue;
    if (p.p === "val") { if (p.n >= seg.values.length) return; seenVal[p.n]++; continue; }
    if (p.n >= seg.tags.length) return;
    if (p.p === "open") { if (!seg.tags[p.n].pair) return; seenTag[p.n]++; stack.push(p.n); }
    else if (p.p === "close") { if (stack.pop() !== p.n) return; }
    else { if (seg.tags[p.n].pair) return; seenTag[p.n]++; }
  }
  return !stack.length && seenTag.every((n) => n === 1) && seenVal.every((n) => n === 1) && (!seg.attr || !seg.tags.length) ? parts : undefined;
}

type Edit = [[number, number], string];
function splice(src: string, [lo, hi]: [number, number], edits: Edit[]): string {
  let out = "", at = lo;
  for (const [[s, e], v] of edits) if (s >= at && e <= hi) { out += src.slice(at, s) + v; at = e; }
  return out + src.slice(at, hi);
}

/** Translate `html`: segments whose lookup returns a valid translation (same placeholders) are replaced, the rest
 *  is left byte for byte. `lookup` gets hashed segments. */
export function translateHtml(html: string, opts: SegmentOpts, lookup: (s: Segment) => string | undefined): string {
  const segs = segmentHashed(html, opts);
  const attrEdits: Edit[] = [];
  const textTr: [Segment, Part[]][] = [];
  for (const seg of segs) {
    const t = lookup(seg);
    const parts = t === undefined ? undefined : check(seg, t);
    if (!parts) continue;
    if (seg.attr) {
      const v = parts.map((p) => (p.p === "val" ? seg.values[p.n].value : p.p === "text" ? p.s : "")).join("");
      attrEdits.push([seg.span, `${seg.attr}="${escapeAttr(v)}"`]);
    } else textTr.push([seg, parts]);
  }
  attrEdits.sort((a, b) => a[0][0] - b[0][0]);
  const edits: Edit[] = textTr.map(([seg, parts]) => {
    let out = "";
    for (const p of parts) {
      if (p.p === "text") out += escapeText(p.s);
      else if (p.p === "val") out += escapeText(seg.values[p.n].value);
      else {
        const tag = seg.tags[p.n];
        out += splice(html, tag.pair ? (p.p === "close" ? tag.close : tag.open) : tag.range, attrEdits);
      }
    }
    return [seg.span, out];
  });
  const covered = edits.map((e) => e[0]);
  for (const e of attrEdits) if (!covered.some(([s, t]) => s <= e[0][0] && e[0][1] <= t)) edits.push(e);
  edits.sort((a, b) => a[0][0] - b[0][0]);
  return splice(html, [0, html.length], edits);
}
