// glot universal website tag → web/public/widget/glot.js (IIFE, < 15 kB gz). One tag on any public site (Wix,
// Squarespace, Bubble, Lovable, Carrd, Webflow, hand-written HTML…), no proxy, no DNS:
//   <script src="https://app.glotlang.com/widget/glot.js" data-project="slug" data-key="pk_…" async></script>
// Optional: data-server (glot API, default api.<host of this script>), data-cdn (locales manifest, default this
// script's origin), data-position (overrides the project setting), data-switcher="none" (your own switcher:
// `glot.setLocale("es")`), data-flags="off", data-wait (ms the page may stay hidden, default 1000, 0 = never).
// A `[data-glot-switcher]` element on the page receives the switcher.
// - Does nothing until the project turns the tag on (`website_snippet.on`, read from `/edge/v1/{p}/status`,
//   cached: switching it off reaches every visitor within a minute). Never blocks rendering: async, every glot
//   request may fail (the page stays as published), last good status / manifest / TM kept in localStorage.
// - No visible wait: a returning visitor is translated from localStorage as the HTML streams in (the observer
//   patches each block before its first paint, no request needed). A first visit that needs a translation hides
//   the page while it is still loading — only then, never a page already on screen — until the TM is applied or
//   `data-wait` ms have passed, whichever comes first (fails open: glot slow or down = the page as published).
//   An async script may run after the first paint; the dashboard's tag therefore starts with a tiny inline
//   script (no request) that puts the same veil (`<style id="glot-veil">`, 1 s at most) when localStorage says
//   this visitor reads one of the offered languages (`glot-w:langs`, written here while the tag is on).
// - Language: `?lang=`, else the visitor's saved choice, else the browser's; `<html lang dir>` follow it.
// - Translation in the browser with the project's TM (patcher.ts: same segmenter and hashes as the server), title,
//   meta description, alt, placeholders and aria labels included, kept up to date across SPA navigations.
// - Discovery (public pages only): text the TM lacks goes to `/edge/v1/{p}/missing` (`mode: "website"`) — never
//   with a session cookie, never on private-looking paths (`/account`, `/cart`, `/checkout`…: a session cookie may
//   be HttpOnly) unless the project includes them, never personal data (masked / not discoverable, re-checked by
//   the server), never what a form shows besides its labels, buttons and placeholders, never the query string. No
//   cookie is read or sent to glot.
// - Metering: one `/usage` unit per translated page view (document load in a non-source language; `data-key`
//   required) against the plan's `website_pages` quota. Past the limit the tag leaves the site as published for
//   10 minutes, then tries again.
// - SEO: client-side translation is not indexed reliably. When the project has SEO-ready glot Edge domains
//   (subdomain CNAME, `status.hreflang`), the switcher sends visitors there and the source page gets hreflang links
//   (also shown in the dashboard to paste into the site's <head>, which is what search engines should see).
import { startPatcher } from "./patcher.ts";
// Defines <glot-locale-switcher> (its menu variant is the switcher shown here).
import { match as pick } from "../../runtime/src/locale-switcher.ts";

type Status = {
  enabled?: boolean;
  selectors?: { exclude?: string[] };
  website?: { on?: boolean; position?: string; locales?: string[]; flags?: boolean };
  discover?: { include?: string[]; exclude?: string[] };
  hreflang?: Record<string, string>;
};
type Manifest = { source: string; locales: { code: string; dir: string }[] };

const me = document.currentScript as HTMLScriptElement | null;
const d = me?.dataset ?? {};
const own = me?.src ? new URL(me.src) : new URL(location.href);
const server = (d.server ?? `${own.protocol}//${own.host.replace(/^app\./, "api.")}`).replace(/\/+$/, "");
const cdn = (d.cdn ?? own.origin).replace(/\/+$/, "");
const project = encodeURIComponent(d.project ?? "");
const api = `${server}/edge/v1/${project}/`;
const html = document.documentElement;
const STORE = "glot-locale"; // the switcher's key
const OVER = "glot-w:over", LANGS = "glot-w:langs";
let storage: Storage | null = null;
try { storage = localStorage; } catch { /* blocked */ }
const get = (k: string) => { try { return storage?.getItem(k) ?? null; } catch { return null; } };
const put = (k: string, v: string) => { try { storage?.setItem(k, v); } catch { /* quota */ } };

/** Last good copy of a glot JSON (null when never seen). */
const local = <T,>(url: string): T | null => { try { return JSON.parse(get(`glot-w:${url}`) ?? "null"); } catch { return null; } };
/** Fresh copy, remembered; null when glot does not answer. */
async function net<T>(url: string): Promise<T | null> {
  try {
    const r = await fetch(url, { credentials: "omit" });
    if (r.ok) { const j = await r.json(); put(`glot-w:${url}`, JSON.stringify(j)); return j; }
  } catch { /* unreachable */ }
  return null;
}

// Anti-flicker veil: opacity keeps the layout (no shift), and it always comes off after `wait` ms.
const wait = +(d.wait ?? 1000);
let veil = document.getElementById("glot-veil"); // put by the inline part of the tag, with its own timeout
const show = () => veil?.remove();
const hide = () => {
  // A page already on screen is never blanked: only one still loading (its first paint may not have happened).
  if (veil || !(wait > 0) || document.readyState !== "loading") return;
  veil = document.createElement("style");
  veil.textContent = "html{opacity:0!important}";
  (document.head ?? html).append(veil);
  setTimeout(show, wait);
};
const dom = (f: () => void) => (document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", f) : f());

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

function switcher(pos: string, offered: string[], flags: boolean) {
  if (d.switcher === "none" || pos === "none" || codes.length < 2) return;
  const s = document.createElement("glot-locale-switcher");
  const at = { project: d.project ?? "", cdn, min: "0", strategy: "event", locales: offered.join(","), flags: flags ? "" : "off", "data-glot-skip": "" };
  for (const [k, v] of Object.entries(at)) if (v || k === "data-glot-skip") s.setAttribute(k, v);
  s.addEventListener("glot:locale-change", (e) => { e.preventDefault(); setLocale((e as CustomEvent).detail.locale); });
  const corner = CORNERS[pos];
  let box: HTMLElement = s;
  if (corner) {
    // Floating: the site's own font and colors (its body's, when it paints a background), not ours. Fixed = no
    // layout shift. Inside an element of the page the switcher inherits everything (font: inherit, currentColor).
    const cs = getComputedStyle(document.body);
    const bg = [cs.backgroundColor, getComputedStyle(html).backgroundColor].find((c) => !/^transparent$|, 0\)$/.test(c));
    box = document.createElement("div");
    box.setAttribute("data-glot-skip", "");
    box.style.cssText = `position:fixed;${corner};z-index:2147483646;font:14px ${cs.fontFamily};color:${bg ? cs.color : "#111"};background:${bg ?? "#fff"};border-radius:6px;box-shadow:0 2px 10px rgb(0 0 0/.15)`;
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

// Same list as the server (`website_setup::PRIVATE_PATHS`), which re-checks every report.
const PRIVATE = /^\/(account|my-account|members?|members-area|profile|checkouts?|cart|basket|orders?|dashboard|admin|wp-admin|billing|settings|users?|customers?)(\/|$)/i;
const globs = (g: string[] = []) => g.map((x) => new RegExp(`^${x.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`));

function go(status: Status, m: Manifest) {
  if (!status.website?.on) return put(LANGS, ""), show(); // tag off: the page stays exactly as published
  source = m.source;
  bases = status.hreflang ?? {};
  const offered = (status.website.locales ?? []).filter(Boolean);
  codes = m.locales.map((l) => l.code).filter((c) => c === source || !offered.length || offered.includes(c));
  dirs = Object.fromEntries(m.locales.map((l) => [l.code, l.dir]));
  put(LANGS, codes.filter((c) => c !== source).map((c) => c.split(/[-_]/)[0].toLowerCase()).join(" "));
  // Served by glot Edge (already translated there): the tag only shows the switcher.
  edgePage = !!document.querySelector('meta[name="glot-tm"]') || Object.entries(bases).some(([l, b]) => l !== source && under(b));
  const fromUrl = match(new URLSearchParams(location.search).get("lang"));
  if (edgePage) current = match(html.lang) ?? source;
  else current = fromUrl ?? match(get(STORE)) ?? navigator.languages.map(match).find(Boolean) ?? source;
  if (fromUrl || edgePage) put(STORE, current);

  const inc = globs(status.discover?.include), exc = globs(status.discover?.exclude);
  let metered = false;
  translate = (l: string) => {
    current = l;
    html.lang = l;
    html.dir = dirs[l] === "rtl" ? "rtl" : "ltr";
    const tmUrl = `${api}${encodeURIComponent(l)}/tm`;
    if (!get(`glot-tm:${tmUrl}`)) hide(); // nothing cached for this language yet: no flash of the source text
    const p = startPatcher({
      source,
      tmUrl,
      storage,
      skip: status.selectors?.exclude,
      missing: `${api}missing`,
      all: true,
      report: { key: d.key, mode: "website" },
      // New text is usually translated within seconds: look again soon, then back off.
      poll: [4000, 15_000, 60_000, 300_000],
      // Never while the HTML is still arriving: the parser shows a sentence cut by a network chunk half-read.
      zone: (p) => document.readyState !== "loading" && !exc.some((r) => r.test(p)) && (inc.length ? inc.some((r) => r.test(p)) : !PRIVATE.test(p)),
      ready: show,
    });
    if (document.readyState === "loading") dom(p.scan); // translated as it streamed in; what is missing is reported now
    if (metered) return;
    metered = true;
    fetch(`${api}usage`, { method: "POST", body: JSON.stringify({ key: d.key, pages: 1, mode: "website" }), keepalive: true })
      .then((r) => { if (r.status === 402) put(OVER, `${Date.now()}`); })
      .catch(() => {});
  };
  const target = current;
  current = source;
  if (!edgePage && target !== source) translate(target);
  else (current = target), show();
  dom(() => {
    switcher(d.position ?? status.website!.position ?? "bottom-right", offered, d.flags !== "off" && status.website!.flags !== false);
    hreflang();
  });
}

// Runs as soon as the script does (not at DOMContentLoaded): with everything in localStorage the page is
// translated while it is being parsed.
if (project && d.key && !(Date.now() - +(get(OVER) ?? 0) < 600_000)) {
  const S = `${api}status`, M = `${cdn}/cdn/${project}/locales.json?min=0`;
  const st = local<Status>(S), m = local<Manifest>(M);
  const fresh = Promise.all([net<Status>(S), net<Manifest>(M)]); // in parallel; for this view only when nothing is cached
  if (st && m) go(st, m); // last known right away; the fresh ones are for the next page view
  else {
    // First visit: hide only when the page says it is in a language this visitor does not read.
    const a = html.lang.split("-")[0].toLowerCase(), b = navigator.language.split("-")[0].toLowerCase();
    if ((a && a !== b) || /[?&]lang=/.test(location.search)) hide();
    void fresh.then(([s, mm]) => (s ?? st) && (mm ?? m) ? go((s ?? st)!, (mm ?? m)!) : show());
  }
} else show();
