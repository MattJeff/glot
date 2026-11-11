// The JS runtimes' page translator (worker, Next proxy, Express): the same pass as the Rust proxy
// (crates/server/src/edge/proxy.rs `translate`): segments (segment.ts) looked up in ONE TM snapshot by
// `<role>|<page template>`, framework payloads (hydration.ts), the SEO rewrite (port of glot_core::edge::seo),
// then the page version and the client patcher tag (same-origin `/__glot/*`, served by edge.ts `own`).
import { translateHtml, tokenize, collapseWs, hash16, template, type Attr } from "./segment.ts";
import { translatePayloads, injectHead, patcherTag, tmInline, TM_INLINE_MAX } from "./hydration.ts";
import { lookup, type Missing, type Tm, type TranslateResult, type Translator } from "./edge.ts";

export const translate: Translator = (html, tm, c) => {
  const tpl = template(c.path, tm.locale);
  let total = 0, translated = 0;
  const missing: Missing[] = [];
  const seen: { hash: string; role: string }[] = [];
  // Plain-text segments (no inline tags / values): framework payload strings equal to them are translated too.
  const payload = new Map<string, string>();
  let out = translateHtml(html, { sourceLocale: c.source, skip: c.skip }, (s) => {
    total++;
    seen.push({ hash: s.hash, role: s.role });
    const v = lookup(tm, s.hash, `${s.role}|${tpl}`);
    if (v === undefined) {
      if (s.discoverable) missing.push({ hash: s.hash, text: s.text, role: s.role });
      return undefined;
    }
    translated++;
    if (!s.tags.length && !s.values.length && !s.attr) payload.set(s.text, v);
    return v;
  });
  out = translatePayloads(out, (src) => payload.get(src)).html;
  out = seo(out, {
    source: c.source,
    locale: tm.locale,
    bases: subdirBases(c.url.origin, c.source, c.locales),
    path: c.path + c.url.search,
    tr: (s) => lookup(tm, hash16(`${c.source}\u0000${collapseWs(s)}`), `text|${tpl}`),
  });
  out = injectHead(out, `<meta name="glot-tm" content="${esc(`${tm.locale}@${tm.v}`)}">`);
  out = injectHead(out, inlined(tm, c.skip));
  out = injectHead(out, patcherTag("/__glot/patcher.js", `/__glot/tm.json?locale=${encodeURIComponent(tm.locale)}`, c.source, c.discover ? "/__glot/missing" : undefined));
  return { html: out, total, translated, missing, seen } satisfies TranslateResult;
};

/** The inline TM tag of a snapshot (+ the project's excluded selectors, as `/__glot/tm.json` serves it), built
 * once per snapshot (snapshots are immutable per version); "" for a TM too large to inline. */
const inlineCache = new WeakMap<object, [string, string]>();
function inlined(tm: Tm, skip: string[]): string {
  const k = skip.join("\u0000");
  const hit = inlineCache.get(tm);
  if (hit?.[0] === k) return hit[1];
  // cheap bound first: a TM with more entries than bytes allowed can't fit
  const tag = Object.keys(tm.t).length > TM_INLINE_MAX / 8 ? "" : tmInline(JSON.stringify(skip.length ? { ...tm, skip } : tm));
  inlineCache.set(tm, [k, tag]);
  return tag;
}

// ------------------------------------------------------------------------------------------- SEO

export interface SeoCtx {
  source: string;
  /** Locale this page is served in. */
  locale: string;
  /** `[locale, base URL]` of every READY locale, the source first; a locale missing here gets no hreflang. */
  bases: [string, string][];
  /** Path + query of the page in source terms. */
  path: string;
  /** Translates one JSON-LD free text; undefined keeps the source. */
  tr: (text: string) => string | undefined;
}

/** Subdirectory mode bases: the source at the origin, `{origin}/{locale}` for the others. */
export const subdirBases = (origin: string, source: string, locales: readonly string[]): [string, string][] =>
  [source, ...locales.filter((l) => l !== source)].map((l) => [l, l === source ? origin : `${origin}/${l}`]);

const RTL = /^(ar|he|iw|fa|ur|ps|sd|ug|yi|dv|ckb)$/;
export const dir = (l: string) => (RTL.test(l.split(/[-_]/)[0]) ? "rtl" : "ltr");
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

/** `https://x.app/es` → `/es`; `https://x.app` → ``; undefined if not absolute. */
const urlPath = (b: string) => { const i = b.indexOf("://"); if (i < 0) return undefined; const r = b.slice(i + 3), j = r.indexOf("/"); return (j < 0 ? "" : r.slice(j)).replace(/\/+$/, ""); };
const hostOf = (b: string) => { const i = b.indexOf("://"); return (i < 0 ? b : b.slice(i + 3)).split("/")[0]; };
const originOf = (b: string) => b.slice(0, b.length - (urlPath(b) ?? "").length);
/** `href` minus the directory prefix `d`, only at a path boundary (`/es` matches `/es`, `/es/x`, `/es?q`, not `/estate`). */
function stripDir(href: string, d: string): string | undefined {
  const p = d.replace(/\/+$/, "");
  if (!href.startsWith(p)) return undefined;
  const rest = href.slice(p.length);
  return rest === "" || "/?#".includes(rest[0]) ? rest : undefined;
}
/** Static files keep their URL (`/logo.png`, `/_next/…`): only pages are localized. */
function isAsset(path: string): boolean {
  const p = path.split(/[?#]/)[0];
  if (p.startsWith("/_next/") || p.startsWith("/api/") || p.startsWith("/cdn-cgi/")) return true;
  const last = p.slice(p.lastIndexOf("/") + 1), dot = last.lastIndexOf(".");
  return dot >= 0 && !["html", "htm", "php", "aspx"].includes(last.slice(dot + 1).toLowerCase());
}

const base = (s: SeoCtx, l: string) => s.bases.find(([x]) => x === l)?.[1].replace(/\/+$/, "");
const urlFor = (s: SeoCtx, l: string, path: string) => { const b = base(s, l); return b === undefined ? undefined : b + (path || "/"); };
/** Source-terms path of a link to this site (any locale's base); undefined for external links, fragments, assets. */
export function sitePath(s: SeoCtx, href0: string): string | undefined {
  const href = href0.trim();
  let path: string | undefined;
  if (href.startsWith("/") && !href.startsWith("//")) {
    path = href;
    for (const [, b] of s.bases) { const pre = urlPath(b); const rest = pre ? stripDir(href, pre) : undefined; if (rest !== undefined) path = rest; }
  } else {
    for (const [, b] of [...s.bases].sort((x, y) => y[1].length - x[1].length)) if ((path = stripDir(href, b)) !== undefined) break;
    if (path === undefined) return undefined;
  }
  path ||= "/";
  return isAsset(path) ? undefined : path;
}
/** Internal link for the served locale: root-relative stays root-relative (subdirectory: gains the prefix). */
export function localizeHref(s: SeoCtx, href: string): string | undefined {
  const path = sitePath(s, href), b = base(s, s.locale);
  if (path === undefined || b === undefined) return undefined;
  if (!href.trim().startsWith("/")) return b + path;
  const src = base(s, s.source) ?? "";
  if (hostOf(b) !== hostOf(src)) return path;
  return (urlPath(b) ?? "") + (stripDir(path, urlPath(src) ?? "") ?? path);
}

const LD_TEXT = ["name", "alternateName", "description", "headline", "alternativeHeadline", "abstract", "text", "caption", "slogan",
  "disambiguatingDescription", "articleBody", "reviewBody", "keywords"];
const LD_URL = ["url", "@id", "mainEntityOfPage", "item", "target", "sameAs"];
function ld(v: unknown, s: SeoCtx, key?: string): unknown {
  if (Array.isArray(v)) return v.map((x) => ld(x, s, key));
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    for (const k of Object.keys(o)) o[k] = k === "inLanguage" ? s.locale : ld(o[k], s, k);
    return o;
  }
  if (typeof v !== "string" || !key) return v;
  if (LD_TEXT.includes(key)) return s.tr(v) ?? v;
  if (LD_URL.includes(key)) { const p = sitePath(s, v); return (p !== undefined && urlFor(s, s.locale, p)) || v; }
  return v;
}

/**
 * International SEO of one page (glot_core::edge::seo::rewrite): `<html lang dir>`, one canonical (the origin's,
 * localized), reciprocal hreflang for every ready locale + x-default, og:locale / og:url, JSON-LD texts through the
 * TM and its URLs localized, internal links localized. Origin-managed alternates (`?lang=en`) stay, made absolute.
 */
export function seo(html: string, s: SeoCtx): string {
  const edits: [number, number, string][] = [];
  // Attribute values as written (lol_html does not decode them either: `&amp;` in an href stays `&amp;`).
  const raw = (x: Attr) => { const m = /=\s*(?:"([^"]*)|'([^']*)|([^\s>]*))/.exec(html.slice(x.range[0], x.range[1])); return m ? (m[1] ?? m[2] ?? m[3]) : ""; };
  const managed = (l: string) => l.toLowerCase() === "x-default" || s.bases.some(([b]) => b.toLowerCase() === l.toLowerCase());
  let canonical: string | undefined, headEnd: number | undefined, ldNext = false;
  for (const t of tokenize(html)) {
    if (t.t === "end") { if (t.name === "head" && headEnd === undefined) headEnd = t.range[0]; continue; }
    if (t.t === "text" && ldNext) {
      const src = html.slice(t.range[0], t.range[1]);
      try { edits.push([t.range[0], t.range[1], JSON.stringify(ld(JSON.parse(src.trim()), s)).replace(/<\//g, "<\\/")]); } catch { /* broken JSON-LD: as is */ }
    }
    ldNext = false;
    if (t.t !== "start") continue;
    const a = t.attrs, get = (n: string) => { const x = a.find((y) => y.name === n); return x && raw(x); };
    const mods = new Map<string, string>(), set = (n: string, v: string) => mods.set(n, v);
    if (t.name === "html") { set("lang", s.locale); set("dir", dir(s.locale)); }
    else if (t.name === "script") ldNext = get("type") === "application/ld+json";
    else if (t.name === "link" && get("rel") !== undefined) {
      const rels = get("rel")!.toLowerCase().split(/\s+/), href = get("href"), lang = get("hreflang");
      if (rels.includes("canonical")) {
        if (href !== undefined) canonical ??= sitePath(s, href) ?? s.path;
        edits.push([t.range[0], t.range[1], ""]);
        continue;
      }
      if (rels.includes("alternate") && lang !== undefined) {
        if (managed(lang)) { edits.push([t.range[0], t.range[1], ""]); continue; }
        if (href?.startsWith("/") && !href.startsWith("//")) set("href", originOf(base(s, s.source) ?? "") + href);
      }
    } else if (t.name === "meta") {
      const p = get("property"), c = get("content");
      if (p === "og:locale") set("content", s.locale.replace(/-/g, "_"));
      else if (p === "og:url" && c !== undefined) { const sp = sitePath(s, c), u = sp === undefined ? undefined : urlFor(s, s.locale, sp); if (u) set("content", u); }
    } else if (t.name === "a" && get("hreflang") === undefined && get("data-glot-skip") === undefined) {
      const h = get("href"), l = h === undefined ? undefined : localizeHref(s, h);
      if (l !== undefined) set("href", l);
    }
    if (!mods.size) continue;
    // lol_html re-serializes a modified start tag: raw attributes joined by one space, modified ones `name="v"`.
    const q = (n: string, v: string) => `${n}="${v.replace(/"/g, "&quot;")}"`;
    const parts = a.map((x) => {
      const v = mods.get(x.name);
      if (v === undefined) return html.slice(x.range[0], x.range[1]);
      mods.delete(x.name);
      return q(html.slice(x.range[0], x.range[0] + x.name.length), v);
    });
    for (const [n, v] of mods) parts.push(q(n, v));
    edits.push([t.range[0], t.range[1], `<${html.slice(t.range[0] + 1, t.range[0] + 1 + t.name.length)}${parts.map((x) => " " + x).join("")}${t.selfClosing ? " />" : ">"}`]);
  }
  if (headEnd !== undefined) {
    const p = canonical ?? s.path, c = urlFor(s, s.locale, p);
    const alt = s.bases.map(([l]) => [l, urlFor(s, l, p)!]);
    const x = urlFor(s, s.source, p);
    if (x !== undefined) alt.push(["x-default", x]);
    edits.push([headEnd, headEnd, (c ? `<link rel="canonical" href="${esc(c)}">` : "") + alt.map(([l, u]) => `<link rel="alternate" hreflang="${esc(l)}" href="${esc(u)}">`).join("")]);
  }
  edits.sort((x, y) => x[0] - y[0]);
  let out = "", at = 0;
  for (const [a, b, v] of edits) if (a >= at) { out += html.slice(at, a) + v; at = b; }
  return out + html.slice(at);
}
