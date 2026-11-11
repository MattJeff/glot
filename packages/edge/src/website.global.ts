// glot universal website tag → web/public/widget/glot.js (IIFE, < 15 kB gz). One tag on any public site (Wix,
// Squarespace, Bubble, Lovable, Carrd, Webflow, hand-written HTML…), no proxy, no DNS:
//   <script src="https://app.glotlang.com/widget/glot.js" data-project="slug" data-key="pk_…" async></script>
// Optional: data-server (glot API, default api.<host of this script>), data-cdn (locales manifest, default this
// script's origin), data-position (overrides the project setting), data-switcher="none" (your own switcher:
// `glot.setLocale("es")`). A `[data-glot-switcher]` element on the page receives the switcher.
// - Does nothing until the project turns the tag on (`website_snippet.on`, read from `/edge/v1/{p}/status`,
//   cached: switching it off reaches every visitor within a minute). Never blocks rendering: async, every glot
//   request may fail (the page stays as published), last good status / manifest / TM kept in localStorage.
// - Language: `?lang=`, else the visitor's saved choice, else the browser's; `<html lang dir>` follow it.
// - Translation in the browser with the project's TM (patcher.ts: same segmenter and hashes as the server), title
//   and meta description included, kept up to date across SPA navigations (MutationObserver).
// - Discovery (public pages only): text the TM lacks goes to `/edge/v1/{p}/missing` (`mode: "website"`) — never
//   with a session cookie, never personal data (masked / not discoverable, re-checked by the server), never what a
//   form shows besides its labels, never the query string. No cookie is read or sent to glot.
// - SEO: client-side translation is not indexed reliably. When the project has SEO-ready glot Edge domains
//   (subdomain CNAME, `status.hreflang`), the switcher sends visitors there and the source page gets hreflang links
//   (also shown in the dashboard to paste into the site's <head>, which is what search engines should see).
import { startPatcher } from "./patcher.ts";
// Defines <glot-locale-switcher> (its menu variant is the switcher shown here).
import { match as pick } from "../../runtime/src/locale-switcher.ts";

type Status = {
  enabled?: boolean;
  selectors?: { exclude?: string[] };
  website?: { on?: boolean; position?: string; locales?: string[] };
  hreflang?: Record<string, string>;
};
type Manifest = { source: string; locales: { code: string; dir: string }[] };

const me = document.currentScript as HTMLScriptElement | null;
const d = me?.dataset ?? {};
const own = me?.src ? new URL(me.src) : new URL(location.href);
const server = (d.server ?? `${own.protocol}//${own.host.replace(/^app\./, "api.")}`).replace(/\/+$/, "");
const cdn = (d.cdn ?? own.origin).replace(/\/+$/, "");
const project = encodeURIComponent(d.project ?? "");
const STORE = "glot-locale"; // the switcher's key
let storage: Storage | null = null;
try { storage = localStorage; } catch { /* blocked */ }
const get = (k: string) => { try { return storage?.getItem(k) ?? null; } catch { return null; } };
const put = (k: string, v: string) => { try { storage?.setItem(k, v); } catch { /* quota */ } };

/** JSON from glot, the last good copy when glot does not answer (null when never seen). */
async function cached<T>(url: string): Promise<T | null> {
  const k = `glot-w:${url}`;
  try {
    const r = await fetch(url, { credentials: "omit" });
    if (r.ok) { const j = await r.json(); put(k, JSON.stringify(j)); return j; }
  } catch { /* unreachable */ }
  try { return JSON.parse(get(k) ?? "null"); } catch { return null; }
}

let current = "", source = "", bases: Record<string, string> = {}, dirs: Record<string, string> = {}, edgePage = false;
let codes: string[] = [];
let translate = (_: string) => {};

const match = (l?: string | null) => pick(l, codes);
/** This page belongs to `base` (same origin, under its path). */
const under = (base: string) => {
  const b = new URL(base), p = b.pathname.replace(/\/$/, "");
  return b.origin === location.origin && (!p || location.pathname === p || location.pathname.startsWith(p + "/"));
};
/** Path of this page in source terms (a subdirectory base's `/es` prefix removed). */
const sourcePath = () => {
  let p = location.pathname;
  for (const b of Object.values(bases)) {
    const bp = new URL(b).pathname.replace(/\/$/, "");
    if (bp && under(b)) p = p.slice(bp.length) || "/";
  }
  return p;
};
const noLang = () => { const u = new URL(location.href); u.searchParams.delete("lang"); return u.search + u.hash; };

function setLocale(code: string) {
  const l = match(code);
  if (!l || l === current) return;
  put(STORE, l);
  // A locale with an SEO-ready Edge domain is served there (indexable, translated server-side).
  const base = bases[l];
  if (base && !under(base)) return location.assign(base.replace(/\/$/, "") + sourcePath() + noLang());
  if (edgePage) { // on an Edge domain, a language it has no domain for: back to the source site, translated there
    const src = bases[source];
    if (src) location.assign(`${src.replace(/\/$/, "")}${sourcePath()}?lang=${encodeURIComponent(l)}`);
    return;
  }
  if (current === source) return translate(l); // in place, no reload
  // ponytail: back to the source text (or another language) = reload; the patcher only goes forward.
  history.replaceState(history.state, "", location.pathname + noLang());
  location.reload();
}
const w = globalThis as unknown as { glot?: Record<string, unknown> };
w.glot = Object.assign(w.glot ?? {}, { setLocale, get locale() { return current; } });

const CORNERS: Record<string, string> = { "bottom-right": "bottom:16px;right:16px", "bottom-left": "bottom:16px;left:16px", "top-right": "top:16px;right:16px", "top-left": "top:16px;left:16px" };

function switcher(pos: string, offered: string[]) {
  if (d.switcher === "none" || pos === "none" || codes.length < 2) return;
  const s = document.createElement("glot-locale-switcher");
  const at = { project: d.project ?? "", cdn, min: "0", strategy: "event", locales: offered.join(","), "data-glot-skip": "" };
  for (const [k, v] of Object.entries(at)) if (v || k === "data-glot-skip") s.setAttribute(k, v);
  s.addEventListener("glot:locale-change", (e) => { e.preventDefault(); setLocale((e as CustomEvent).detail.locale); });
  const corner = CORNERS[pos];
  let box: HTMLElement = s;
  if (corner) {
    box = document.createElement("div");
    box.setAttribute("data-glot-skip", "");
    box.style.cssText = `position:fixed;${corner};z-index:2147483646;font:14px system-ui,sans-serif;color:#111;background:#fff;border-radius:6px;box-shadow:0 2px 10px rgb(0 0 0/.15)`;
    box.append(s);
  }
  // A selector the page renders late (SPA header): placed as soon as it exists, put back if a re-render drops it.
  const place = () => {
    if (box.isConnected) return;
    const host = document.querySelector("[data-glot-switcher]") ?? (corner ? document.body : (() => { try { return document.querySelector(pos); } catch { return null; } })());
    host?.append(box);
  };
  place();
  new MutationObserver(place).observe(document.body, { childList: true, subtree: true });
}

/** hreflang alternates on the source site (what the dashboard tells you to paste in <head>, for crawlers). */
function hreflang() {
  const src = bases[source];
  if (edgePage || !src || !under(src) || Object.keys(bases).length < 2) return;
  if (document.querySelector('link[rel=alternate][hreflang]:not([data-glot])')) return; // pasted by the site: theirs
  const sync = () => {
    document.querySelectorAll("link[data-glot]").forEach((l) => l.remove());
    const p = sourcePath() + location.search.replace(/[?&]lang=[^&]*/, "").replace(/^&/, "?");
    for (const [l, b] of [...Object.entries(bases), ["x-default", src]]) {
      const link = Object.assign(document.createElement("link"), { rel: "alternate", hreflang: l, href: b.replace(/\/$/, "") + p });
      link.setAttribute("data-glot", "");
      document.head.append(link);
    }
  };
  sync();
  for (const k of ["pushState", "replaceState"] as const) {
    const f = history[k];
    history[k] = function (this: History, ...a: Parameters<History["pushState"]>) { const r = f.apply(this, a); sync(); return r; };
  }
  addEventListener("popstate", sync);
}

async function boot() {
  if (!project) return;
  const k = `glot-w:${server}/edge/v1/${project}/status`;
  let st: Status | null = null;
  try { st = JSON.parse(get(k) ?? "null"); } catch { /* corrupt */ }
  const fresh = cached<Status>(`${server}/edge/v1/${project}/status`);
  const status = st ?? (await fresh); // last known status right away; the fresh one is for the next page view
  if (!status?.website?.on) return; // tag off (or glot never reached): the page stays exactly as published
  const m = await cached<Manifest>(`${cdn}/cdn/${project}/locales.json?min=0`);
  if (!m) return;
  source = m.source;
  bases = status.hreflang ?? {};
  const offered = (status.website.locales ?? []).filter(Boolean);
  codes = m.locales.map((l) => l.code).filter((c) => c === source || !offered.length || offered.includes(c));
  dirs = Object.fromEntries(m.locales.map((l) => [l.code, l.dir]));
  edgePage = !!document.querySelector('meta[name="glot-tm"]');
  const html = document.documentElement;
  const fromUrl = match(new URLSearchParams(location.search).get("lang"));
  if (edgePage) current = match(html.lang) ?? source;
  else current = fromUrl ?? match(get(STORE)) ?? navigator.languages.map(match).find(Boolean) ?? source;
  if (fromUrl || edgePage) put(STORE, current);

  translate = (l: string) => {
    current = l;
    html.lang = l;
    html.dir = dirs[l] === "rtl" ? "rtl" : "ltr";
    startPatcher({
      source,
      tmUrl: `${server}/edge/v1/${project}/${encodeURIComponent(l)}/tm`,
      storage,
      skip: status.selectors?.exclude,
      missing: d.key ? `${server}/edge/v1/${project}/missing` : undefined,
      all: true,
      report: { key: d.key, mode: "website" },
    });
  };
  const target = current;
  current = source;
  if (!edgePage && target !== source) translate(target);
  else current = target;
  switcher(d.position ?? status.website.position ?? "bottom-right", offered);
  hreflang();
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => void boot());
else void boot();
