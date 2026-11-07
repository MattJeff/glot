// Hydration-safe Edge: translate the strings of the serialized payload that frameworks hydrate from, so the
// client renders the same (translated) text the server HTML shows — no mismatch, no re-render, no flash.
//   Next.js Pages  <script id="__NEXT_DATA__" type="application/json">  (getStaticProps/getServerSideProps props)
//   Next.js App    <script>self.__next_f.push([1,"…"])</script>          (RSC flight: server components' text)
//   Nuxt 3/4       <script type="application/json" id="__NUXT_DATA__">  (devalue array: useFetch/useAsyncData)
// A string is replaced only when it is exactly (after whitespace folding) the source of a translated segment of
// the page, never a flight reference ("$…"), an object key, or the value of an identifier-like key (id, slug,
// href, type, variant…) — app logic comparing such values keeps working.
// What this cannot reach: text hard-coded in client bundles (client components, Pages-router JSX, SvelteKit's
// JS-literal data). There the framework hydrates/re-renders the source text and the patcher re-applies the TM
// in the same task (MutationObserver → no painted flash); React logs one recoverable hydration error for it.
// ponytail: regex-located scripts + JSON walk; a full HTML tokenizer is the upgrade if pages get exotic.

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

/** Deep copy of a JSON value with translatable string values replaced. */
function walk(v: unknown, lookup: Lookup, n: { c: number }, key = ""): unknown {
  if (typeof v === "string") {
    if (ID_KEYS.test(key)) return v;
    const t = tr(v, lookup);
    if (t !== undefined) n.c++;
    return t ?? v;
  }
  if (Array.isArray(v)) return v.map((x) => walk(x, lookup, n, key));
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, walk(x, lookup, n, k)]));
  return v;
}

function nextData(html: string, lookup: Lookup, n: { c: number }) {
  return html.replace(/(<script\b[^>]*\bid="__NEXT_DATA__"[^>]*>)([\s\S]*?)(<\/script>)/, (all, open, body, close) => {
    try {
      return open + esc(JSON.stringify(walk(JSON.parse(body), lookup, n))) + close;
    } catch {
      return all;
    }
  });
}

/** devalue's flat array: values are top-level entries, objects/arrays hold indexes. */
function nuxtData(html: string, lookup: Lookup, n: { c: number }) {
  return html.replace(/(<script\b[^>]*\bid="__NUXT_DATA__"[^>]*>)([\s\S]*?)(<\/script>)/, (all, open, body, close) => {
    try {
      const a = JSON.parse(body) as unknown[];
      if (!Array.isArray(a)) return all;
      const skip = new Set<number>();
      for (const x of a) {
        if (x && typeof x === "object" && !Array.isArray(x)) for (const [k, i] of Object.entries(x)) if (ID_KEYS.test(k) && typeof i === "number") skip.add(i);
        // ["Reactive", 3], ["Set", …]: the leading string is a type tag, not text
        if (Array.isArray(x) && typeof x[0] === "string") for (const i of x.slice(1)) if (typeof i === "number") skip.add(i);
      }
      const out = a.map((x, i) => {
        if (typeof x !== "string" || skip.has(i)) return x;
        const t = tr(x, lookup);
        if (t !== undefined) n.c++;
        return t ?? x;
      });
      return open + esc(JSON.stringify(out)) + close;
    } catch {
      return all;
    }
  });
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
  return { html: out, replaced: n.c };
}

const attr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
/** `<script async>` of the client patcher (glot_core::edge::hydration::patcher_tag); `missing` = discovery endpoint. */
export const patcherTag = (scriptUrl: string, tmUrl: string, sourceLocale: string, missing?: string) =>
  `<script async src="${attr(scriptUrl)}" data-tm="${attr(tmUrl)}" data-source="${attr(sourceLocale)}"${missing ? ` data-missing="${attr(missing)}"` : ""}></script>`;
/** Insert `tag` after `<meta name="glot-tm" …>`, else right after `<head>` (glot_core::edge::hydration::inject_head). */
export function injectHead(html: string, tag: string): string {
  const m = /<meta name="glot-tm"[^>]*>/i.exec(html) ?? /<head(?:\s[^>]*)?>/i.exec(html);
  return m ? html.slice(0, m.index + m[0].length) + tag + html.slice(m.index + m[0].length) : html;
}
