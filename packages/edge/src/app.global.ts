// glot app mode → web/public/widget/glot-app.global.js (IIFE): translate the pages behind sign-in (dashboards,
// accounts) in the browser, from one script tag on the app's own domain:
//   <script async src="https://glotlang.com/widget/glot-app.global.js" data-glot-app data-project="site"
//     data-server="https://api.glotlang.com" data-source="en" data-locales="es,de" data-key="pk_…"></script>
// Runs only when the project's Edge settings say `auth_pages = patcher` (status, no redeploy to switch it off).
// - Same domain, nothing proxied: the app's cookies are never read, set or sent by this script (glot requests
//   are cross-origin, without credentials). Only `glot_locale` (the visitor's language choice) is written.
// - It applies the project's TM (the same one the Edge serves, `/edge/v1/{project}/{locale}/tm`) to text nodes as
//   the app renders them (patcher.ts), in place.
// - Discovery is off unless the project turns `auth_discover` on, and then only for UI text (ALLOW selectors):
//   `[data-glot-ui]` text as text, anything else as a hash alone; never form values, user zones
//   (`[data-glot-user]`, the project's excluded selectors), personal data (masked / not discoverable), and every
//   new segment waits for a review in glot before it is served.
// - Language: `?lang=` (what application-path redirects carry), else the saved choice, else the browser's.
//   A `<select>` switcher goes into `[data-glot-switcher]`, or a corner of the page (`data-switcher="none"`: your
//   own, calling `glotApp.setLocale("es")`).
import { startPatcher } from "./patcher.ts";

const ALLOW = "[data-glot-ui],nav,button,label,h1,h2,h3,h4,h5,h6,th,legend,summary,title,[role=button],[role=tab],[role=menuitem]";
const d = (document.currentScript as HTMLScriptElement | null)?.dataset ?? {};
const source = d.source ?? "en";
const all = [source, ...(d.locales ?? "").split(",").map((l) => l.trim()).filter(Boolean)];
const server = (d.server ?? "https://api.glotlang.com").replace(/\/+$/, "");
const project = encodeURIComponent(d.project ?? "");
let storage: Storage | null = null;
try { storage = localStorage; } catch { /* blocked */ }

const match = (l?: string | null) => {
  const x = l?.toLowerCase();
  return x ? all.find((c) => c.toLowerCase() === x) ?? all.find((c) => c.toLowerCase() === x.split("-")[0]) : undefined;
};
const saved = () => { try { return storage?.getItem("glot_locale"); } catch { return null; } };
const fromUrl = match(new URLSearchParams(location.search).get("lang"));
const locale = fromUrl ?? match(saved() ?? /(?:^|;\s*)glot_locale=([^;]+)/.exec(document.cookie)?.[1]) ?? navigator.languages.map(match).find(Boolean) ?? source;

function setLocale(l: string) {
  const m = match(l);
  if (!m) return;
  try { storage?.setItem("glot_locale", m); } catch { /* blocked */ }
  document.cookie = `glot_locale=${m}; Path=/; Max-Age=31536000; SameSite=Lax`;
  if (m !== locale) location.reload(); // ponytail: back to the source text = reload (the patcher only goes forward)
}
if (fromUrl) setLocale(fromUrl);
(globalThis as { glotApp?: unknown }).glotApp = { locale, locales: all, setLocale };

function switcher() {
  if (d.switcher === "none" || all.length < 2) return;
  const s = document.createElement("select");
  s.setAttribute("data-glot-skip", "");
  s.setAttribute("aria-label", "Language");
  for (const l of all) {
    let name = l;
    try { name = new Intl.DisplayNames([l], { type: "language" }).of(l) ?? l; } catch { /* old browser */ }
    s.add(new Option(name, l, false, l === locale));
  }
  s.onchange = () => setLocale(s.value);
  const host = document.querySelector("[data-glot-switcher]");
  if (host) host.append(s);
  else (s.style.cssText = "position:fixed;right:12px;bottom:12px;z-index:2147483647;font:14px system-ui"), document.body.append(s);
}

type Status = { enabled?: boolean; auth?: { pages?: string; discover?: boolean }; selectors?: { exclude?: string[] } };
async function boot() {
  switcher();
  if (locale === source || !project) return;
  const key = `glot-app-status:${project}`;
  let st: Status | null = null;
  try {
    const r = await fetch(`${server}/edge/v1/${project}/status`, { credentials: "omit" });
    if (r.ok) (st = await r.json()), storage?.setItem(key, JSON.stringify(st));
  } catch { /* glot unreachable: last known status below */ }
  try { st ??= JSON.parse(storage?.getItem(key) ?? "null"); } catch { /* corrupt */ }
  if (!st || st.enabled === false || st.auth?.pages !== "patcher") return; // not enabled for this project: untouched
  document.documentElement.lang = locale;
  startPatcher({
    source,
    tmUrl: `${server}/edge/v1/${project}/${encodeURIComponent(locale)}/tm`,
    storage,
    skip: st.selectors?.exclude,
    missing: st.auth.discover && d.key ? `${server}/edge/v1/${project}/missing` : undefined,
    app: { allow: ALLOW },
    report: { key: d.key, auth: true, mode: "app" },
  });
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => void boot());
else void boot();
