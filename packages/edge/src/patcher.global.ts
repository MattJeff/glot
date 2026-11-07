// Browser entry of the Edge patcher → web/public/widget/edge-patcher.global.js (IIFE). Injected by the Edge as
//   <script async src="/__glot/patcher.js" data-tm="/__glot/tm.json?locale=es" data-source="en" data-missing="/__glot/missing"></script>
// right after `<meta name="glot-tm" content="{locale}@{version}">` (`data-missing`: only when the runtime has the
// project key). Also pins the Edge's SEO head (html lang/dir, canonical, hreflang alternates): an SPA rewriting
// them for its own locale would make the localized page point at the source page.
import { startPatcher } from "./patcher.ts";

/** Keep the Edge's `<html lang dir>`, canonical and hreflang alternates if the app rewrites them. */
function pinHead() {
  const html = document.documentElement, lang = html.getAttribute("lang"), dir = html.getAttribute("dir");
  const q = () => [...document.head.querySelectorAll('link[rel="canonical"],link[rel="alternate"][hreflang]')];
  const links = q().map((l) => l.outerHTML).join("");
  if (!links) return;
  const mo = new MutationObserver(() => {
    if (lang !== null && html.getAttribute("lang") !== lang) html.setAttribute("lang", lang);
    if (dir !== null && html.getAttribute("dir") !== dir) html.setAttribute("dir", dir);
    const now = q();
    if (now.map((l) => l.outerHTML).join("") !== links) {
      now.forEach((l) => l.remove());
      document.head.insertAdjacentHTML("beforeend", links);
    }
    mo.takeRecords();
  });
  mo.observe(html, { attributes: true, attributeFilter: ["lang", "dir"] });
  mo.observe(document.head, { subtree: true, childList: true, attributes: true, attributeFilter: ["href", "hreflang", "rel"] });
}

const d = (document.currentScript as HTMLScriptElement | null)?.dataset ?? {};
// Loaded `async` (never render-blocking): it may run before the rest of <head> is parsed, so the page version,
// the inline TM and the SEO links are read once the document is parsed. Anything the app rendered before that is
// caught up when the TM is in (startPatcher re-translates every run-carrying block).
function boot() {
  const ver = /@(\d+)$/.exec(document.querySelector('meta[name="glot-tm"]')?.getAttribute("content") ?? "");
  let storage: Storage | null = null;
  try { storage = localStorage; } catch { /* blocked */ }
  startPatcher({ source: d.source ?? "en", tmUrl: d.tm, missing: d.missing, inline: document.getElementById("glot-tm")?.textContent, pageVersion: ver ? +ver[1] : 0, storage });
  pinHead();
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
else boot();
