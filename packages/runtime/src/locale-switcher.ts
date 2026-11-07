// <glot-locale-switcher>: a drop-in language switcher fed by the project's public locales manifest
// (`/cdn/{slug}/locales.json`), so a language added in glot shows up without redeploying the site.
// Also `glot.loadCatalog(locale)` for client-rendered apps that load their strings at runtime.
// Built as an IIFE (web/public/widget/locale-switcher.global.js); the pure helpers are exported for tests.

export type Loc = { code: string; name: string; dir: "ltr" | "rtl"; coverage: number };
export type Manifest = { source: string; locales: Loc[]; updated_at: string | null };
type Json = { [k: string]: unknown };

const STORE = "glot-locale";

/** Picks the manifest locale matching `tag` exactly, else by base language (`fr-CA` → `fr`). */
export function match(tag: string | null | undefined, codes: string[]): string | undefined {
  if (!tag) return;
  const t = tag.toLowerCase();
  const base = (c: string) => c.toLowerCase().split(/[-_]/)[0];
  return codes.find((c) => c.toLowerCase() === t) ?? codes.find((c) => base(c) === base(t));
}

/** `auto` resolves to `query:lang` when the URL has `?lang=`, `prefix` when the path starts with a locale, else `event`. */
export function resolveStrategy(strategy: string, url: URL, codes: string[]): string {
  if (strategy && strategy !== "auto") return strategy;
  if (url.searchParams.has("lang")) return "query:lang";
  if (codes.includes(url.pathname.split("/")[1])) return "prefix";
  return "event";
}

/** The locale the page is in, per strategy (`event`: the saved choice); falls back to <html lang>, the saved choice, the source. */
export function currentLocale(strategy: string, url: URL, cookie: string, htmlLang: string, saved: string | null, m: Manifest): string {
  const codes = m.locales.map((l) => l.code);
  const [kind, name = "lang"] = strategy.split(":");
  let found: string | undefined;
  if (kind === "query") found = match(url.searchParams.get(name), codes);
  else if (kind === "prefix") found = codes.find((c) => c === url.pathname.split("/")[1]) ?? m.source;
  else if (kind === "cookie") found = match(new URLSearchParams(cookie.replace(/;\s*/g, "&")).get(name), codes);
  else found = match(saved, codes); // event: the app may cancel the change, the saved choice is what the user picked
  return found ?? match(htmlLang, codes) ?? match(saved, codes) ?? m.source;
}

/** Where to go for `code` under a navigating strategy (`query:*`, `prefix`, `prefix:always`); null otherwise. */
export function targetUrl(strategy: string, url: URL, code: string, m: Manifest): string | null {
  const [kind, opt = "lang"] = strategy.split(":");
  const next = new URL(url.href);
  if (kind === "query") {
    next.searchParams.set(opt, code);
    return next.href;
  }
  if (kind === "prefix") {
    const parts = next.pathname.split("/");
    if (m.locales.some((l) => l.code === parts[1])) parts.splice(1, 1);
    // ponytail: "as-needed" by default (source unprefixed); `prefix:always` prefixes it too.
    if (code !== m.source || opt === "always") parts.splice(1, 0, code);
    next.pathname = parts.join("/") || "/";
    return next.href;
  }
  return null;
}

const script = typeof document !== "undefined" ? (document.currentScript as HTMLScriptElement | null) : null;
const defaults = {
  project: script?.dataset.project ?? "",
  cdn: script?.dataset.cdn ?? (script?.src ? new URL(script.src).origin : ""),
};
const base = (cdn: string) => (cdn || defaults.cdn || location.origin).replace(/\/+$/, "");
// The browser's HTTP cache revalidates with the ETag once `max-age` (60 s) passes; no custom headers, so no
// CORS preflight. One request per URL per page.
const memo = new Map<string, Promise<unknown>>();
const get = <T,>(url: string): Promise<T> => {
  let p = memo.get(url);
  if (!p) {
    p = fetch(url, { credentials: "same-origin" }).then((r) => (r.ok ? r.json() : Promise.reject(new Error(`${r.status} ${url}`))));
    p.catch(() => memo.delete(url));
    memo.set(url, p);
  }
  return p as Promise<T>;
};

/** Fetches a published catalog (nested JSON) and announces it with a `glot:catalog` event on `document`. */
export async function loadCatalog(locale: string, opts: { project?: string; cdn?: string } = {}): Promise<Json> {
  const project = opts.project ?? defaults.project;
  const messages = await get<Json>(`${base(opts.cdn ?? "")}/cdn/${encodeURIComponent(project)}/${encodeURIComponent(locale)}.json`);
  document.dispatchEvent(new CustomEvent("glot:catalog", { detail: { locale, messages } }));
  return messages;
}

const CSS = `:host{display:inline-block;font:inherit;font-size:var(--glot-font-size,.875rem)}
[part=list]{display:inline-flex;gap:var(--glot-gap,2px);padding:var(--glot-list-padding,2px);border:1px solid var(--glot-border,currentColor);border-radius:var(--glot-radius,999px);background:var(--glot-list-bg,transparent)}
button,select{font:inherit;color:var(--glot-fg,inherit);cursor:pointer}
button{border:0;background:var(--glot-bg,transparent);padding:var(--glot-padding,.25em .75em);border-radius:var(--glot-radius,999px)}
button[aria-current=true]{background:var(--glot-active-bg,currentColor)}
button[aria-current=true] span{color:var(--glot-active-fg,Canvas)}
select{background:var(--glot-bg,transparent);border:1px solid var(--glot-border,currentColor);border-radius:var(--glot-radius,6px);padding:var(--glot-padding,.25em .5em)}
:focus-visible{outline:2px solid var(--glot-focus,Highlight);outline-offset:2px}`;

if (typeof customElements !== "undefined" && !customElements.get("glot-locale-switcher")) {
  class GlotLocaleSwitcher extends HTMLElement {
    static observedAttributes = ["project", "cdn", "variant", "labels", "strategy", "min"];
    #root = this.attachShadow({ mode: "open" });
    #m?: Manifest;
    #strategy = "event";

    connectedCallback() {
      this.#load();
    }
    attributeChangedCallback() {
      if (this.isConnected) this.#load();
    }

    async #load() {
      const project = this.getAttribute("project") ?? defaults.project;
      const min = this.getAttribute("min") ?? "0.95";
      try {
        this.#m = await get<Manifest>(`${base(this.getAttribute("cdn") ?? "")}/cdn/${encodeURIComponent(project)}/locales.json?min=${encodeURIComponent(min)}`);
      } catch {
        this.#root.innerHTML = ""; // no manifest: render nothing rather than a broken control
        return;
      }
      this.#render();
    }

    #label(l: Loc, mode: string) {
      if (mode === "code") return l.code.split(/[-_]/)[0].toUpperCase();
      if (l.name !== l.code) return l.name;
      try {
        return new Intl.DisplayNames([l.code], { type: "language" }).of(l.code) ?? l.code;
      } catch {
        return l.code;
      }
    }

    #render() {
      const m = this.#m!;
      const url = new URL(location.href);
      const codes = m.locales.map((l) => l.code);
      this.#strategy = resolveStrategy(this.getAttribute("strategy") ?? "auto", url, codes);
      let saved: string | null = null;
      try {
        saved = localStorage.getItem(STORE);
      } catch {}
      const current = currentLocale(this.#strategy, url, document.cookie, document.documentElement.lang, saved, m);
      const select = this.getAttribute("variant") === "select";
      const labels = this.getAttribute("labels") ?? (select ? "name" : "code");
      const aria = this.getAttribute("aria-label") ?? "Language";
      const root = this.#root;
      root.innerHTML = `<style>${CSS}</style>`;
      if (m.locales.length < 2) return;
      if (select) {
        const s = document.createElement("select");
        s.setAttribute("part", "select");
        s.setAttribute("aria-label", aria);
        for (const l of m.locales) {
          const o = new Option(this.#label(l, labels), l.code, false, l.code === current);
          o.lang = l.code;
          o.dir = l.dir;
          s.append(o);
        }
        s.onchange = () => this.#choose(s.value, current);
        root.append(s);
        return;
      }
      const list = document.createElement("div");
      list.setAttribute("part", "list");
      list.setAttribute("role", "group");
      list.setAttribute("aria-label", aria);
      for (const l of m.locales) {
        const b = document.createElement("button");
        b.type = "button";
        const on = l.code === current;
        b.setAttribute("part", on ? "pill pill-current" : "pill");
        b.setAttribute("aria-current", String(on));
        b.lang = l.code;
        b.dir = l.dir;
        const span = document.createElement("span");
        span.textContent = this.#label(l, labels);
        b.append(span);
        if (labels === "code") b.title = this.#label(l, "name");
        if (labels === "code") b.setAttribute("aria-label", b.title);
        b.onclick = () => this.#choose(l.code, current);
        list.append(b);
      }
      // Arrow keys move between pills (mirrored in RTL), Home/End jump to the ends.
      list.onkeydown = (e) => {
        const bs = [...list.querySelectorAll("button")];
        const i = bs.indexOf(root.activeElement as HTMLButtonElement);
        const rtl = getComputedStyle(this).direction === "rtl";
        const step = { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1 } as Record<string, number>;
        const to = e.key === "Home" ? 0 : e.key === "End" ? bs.length - 1 : step[e.key] ? (i + step[e.key] + bs.length) % bs.length : -1;
        if (to < 0) return;
        e.preventDefault();
        bs[to].focus();
      };
      root.append(list);
    }

    #choose(code: string, previous: string) {
      if (code === previous) return;
      const m = this.#m!;
      const loc = m.locales.find((l) => l.code === code)!;
      try {
        localStorage.setItem(STORE, code);
      } catch {}
      const ev = new CustomEvent("glot:locale-change", { detail: { locale: code, previous, dir: loc.dir }, bubbles: true, composed: true, cancelable: true });
      if (!this.dispatchEvent(ev)) return this.#render(); // the app handled it
      const [kind, name = "lang"] = this.#strategy.split(":");
      if (kind === "cookie") {
        document.cookie = `${name}=${encodeURIComponent(code)}; path=/; max-age=31536000; samesite=lax`;
        return location.reload();
      }
      const to = targetUrl(this.#strategy, new URL(location.href), code, m);
      if (to) return location.assign(to);
      // `event`: the app swaps strings itself (e.g. with glot.loadCatalog); keep <html> and the pills in sync.
      document.documentElement.lang = code;
      document.documentElement.dir = loc.dir;
      this.#render();
    }
  }
  customElements.define("glot-locale-switcher", GlotLocaleSwitcher);
  const w = window as unknown as { glot?: Json };
  w.glot = Object.assign(w.glot ?? {}, { loadCatalog });
}
