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

// Languages are not countries: a flag only where one country is the obvious default for the language. A region
// subtag wins (pt-BR → 🇧🇷, en-US → 🇺🇸); ar, eo, la, ca, sw… get the code badge. Override with `flag-xx` / `.flags`.
// Format: a bare code is its own region (fr → FR), `ab:CD` maps (en → GB).
const FLAGS = Object.fromEntries(
  "fr de es it pt nl ru pl tr fi ro hu bg hr sk lt lv id th is mt mk no en:gb ja:jp ko:kr zh:cn sv:se da:dk nb:no nn:no cs:cz el:gr he:il hi:in uk:ua vi:vn et:ee sl:si ms:my fa:ir bn:bd ur:pk sq:al sr:rs ka:ge hy:am kk:kz fil:ph tl:ph"
    .split(" ")
    .map((e) => (e.includes(":") ? e.split(":") : [e, e])),
);
const ri = (region: string) => String.fromCodePoint(...[...region.toUpperCase()].map((c) => c.charCodeAt(0) + 0x1f1a5));
const RI = /[\u{1f1e6}-\u{1f1ff}]/u;

/** The flag for a locale ("" = none, show the code badge). `over` keys are lowercase codes; values an emoji or a region (`us`). */
export function flagFor(code: string, over: Record<string, string> = {}): string {
  const c = code.toLowerCase();
  const [lang, ...rest] = c.split(/[-_]/);
  const region = rest.find((p) => /^[a-z]{2}$/.test(p));
  const v = over[c] ?? (region ?? (rest.some((p) => /^\d{3}$/.test(p)) ? "" : (over[lang] ?? FLAGS[lang] ?? "")));
  return /^[a-z]{2}$/i.test(v) ? ri(v) : v; // es-419 (a continent) → no flag
}

// Windows has no flag emoji (it draws the two letters): measured once, then the code badge instead.
let emojiFlags: boolean | undefined;
const flagsRender = () =>
  (emojiFlags ??= (() => {
    try {
      const c = document.createElement("canvas").getContext("2d")!;
      return c.measureText(ri("fr")).width < 1.5 * c.measureText(ri("f")).width;
    } catch {
      return false;
    }
  })());
const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
const el = (tag: string, a: Record<string, string> = {}, text = "") => {
  const e = document.createElement(tag);
  for (const k in a) e.setAttribute(k, a[k]);
  e.textContent = text;
  return e;
};

const script = typeof document !== "undefined" ? (document.currentScript as HTMLScriptElement | null) : null;
const defaults = {
  project: script?.dataset.project ?? "",
  cdn: script?.dataset.cdn ?? (script?.src ? new URL(script.src).origin : ""),
};
const base = (cdn: string) => (cdn || defaults.cdn || location.origin).replace(/\/+$/, "");
// The browser's HTTP cache revalidates with the ETag once `max-age` (60 s) passes; no custom headers, so no
// CORS preflight. One request per URL per page. The last good copy is kept in localStorage and used when the CDN
// does not answer (glot down: the switcher and catalogs keep working with what the visitor last saw).
const memo = new Map<string, Promise<unknown>>();
const store = (k: string, v?: string): string | null => {
  try {
    if (v !== undefined) localStorage.setItem(k, v);
    return localStorage.getItem(k);
  } catch {
    return null; // storage blocked / full
  }
};
const get = <T,>(url: string): Promise<T> => {
  let p = memo.get(url);
  if (!p) {
    const key = `glot-cdn:${url}`;
    p = fetch(url, { credentials: "same-origin" })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`${r.status} ${url}`))))
      .then((j) => (store(key, JSON.stringify(j)), j))
      .catch((e) => {
        const last = store(key);
        if (last === null) throw e;
        return JSON.parse(last);
      });
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

const CSS = `:host{--l:color-mix(in srgb,currentColor 20%,transparent);display:inline-block;font:inherit;font-size:var(--glot-font-size,.875rem)}
[part=list]{display:inline-flex;gap:var(--glot-gap,2px);padding:var(--glot-list-padding,2px);border:1px solid var(--glot-border,currentColor);border-radius:var(--glot-radius,999px);background:var(--glot-list-bg,transparent)}
button,select{font:inherit;color:var(--glot-fg,inherit);cursor:pointer}
button{border:0;background:var(--glot-bg,transparent);padding:var(--glot-padding,.25em .75em);border-radius:var(--glot-radius,999px)}
button[aria-current=true]{background:var(--glot-active-bg,currentColor)}
button[aria-current=true] span{color:var(--glot-active-fg,Canvas)}
select{background:var(--glot-bg,transparent);border:1px solid var(--glot-border,currentColor);border-radius:var(--glot-radius,6px);padding:var(--glot-padding,.25em .5em)}
:focus-visible{outline:2px solid var(--glot-focus,Highlight);outline-offset:2px}
[hidden]{display:none!important}.w{position:relative}
[part~=trigger],[role=option]{display:flex;align-items:center;gap:.4em}
[part~=trigger]{border:1px solid var(--glot-border,var(--l));border-radius:var(--glot-radius,6px);padding:var(--glot-padding,.3em .6em)}
[part~=menu]{position:absolute;z-index:var(--glot-z,1000);top:calc(100% + 4px);inset-inline-start:0;min-width:max(100%,13em);max-height:var(--glot-menu-max-height,20em);overflow:auto;box-sizing:border-box;margin:0;padding:4px;background:var(--glot-menu-bg,Canvas);color:var(--glot-menu-fg,CanvasText);border:1px solid var(--l);border-radius:var(--glot-menu-radius,8px);box-shadow:var(--glot-shadow,0 8px 24px rgb(0 0 0/.15));animation:i .12s ease-out;transform-origin:top}
.up{top:auto;bottom:calc(100% + 4px);transform-origin:bottom}
@keyframes i{from{opacity:0;transform:scale(.96)}}@media (prefers-reduced-motion:reduce){[part~=menu]{animation:none}}
ul{list-style:none;margin:0;padding:0}
[role=option]{gap:.6em;padding:.4em .6em;border-radius:4px;cursor:pointer;white-space:nowrap;outline-offset:-2px}
[role=option]:hover,[role=option]:focus{background:var(--glot-hover-bg,var(--l))}
[aria-selected=true]{font-weight:600}[aria-selected=true]:after{content:"✓";margin-inline-start:auto;padding-inline-start:1em}
small{opacity:.6}.b{font-size:.7em;font-weight:600;border:1px solid;border-radius:3px;padding:0 .3em;min-width:2em;text-align:center;opacity:.7}
input{display:block;width:100%;box-sizing:border-box;margin-bottom:4px;font:inherit;color:inherit;background:transparent;border:1px solid var(--l);border-radius:4px;padding:.3em .5em}`;

// An app that mounts over a pre-rendered page (Leptos/React CSR) re-creates the element: an open menu is remembered
// on disconnect and reopened by the instance that renders at the same document position within 2 s.
// ponytail: matched by position among all switchers (attributes may differ between the static and app markup).
let kept: { i: number; q: string; f: number; t: number } | undefined;
let live: HTMLElement | undefined; // the switcher whose menu is open
const nth = (e: HTMLElement) => [...document.querySelectorAll("glot-locale-switcher")].indexOf(e);

if (typeof customElements !== "undefined" && !customElements.get("glot-locale-switcher")) {
  class GlotLocaleSwitcher extends HTMLElement {
    static observedAttributes = ["project", "cdn", "variant", "labels", "strategy", "min", "flags", "locales"];
    #root = this.attachShadow({ mode: "open" });
    #m?: Manifest;
    #strategy = "event";
    #flags: Record<string, string> = {};
    #close = (_keep?: boolean) => {};

    /** Per-locale flag overrides (`{ en: "🇺🇸", ar: "" }`), on top of the `flag-xx` attributes. */
    get flags() {
      return this.#flags;
    }
    set flags(v: Record<string, string>) {
      this.#flags = v ?? {};
      if (this.#m) this.#render();
    }

    connectedCallback() {
      this.#load();
    }
    disconnectedCallback() {
      this.#close(true);
    }
    attributeChangedCallback() {
      if (this.isConnected) this.#load();
    }

    async #load() {
      const project = this.getAttribute("project") ?? defaults.project;
      const min = this.getAttribute("min") ?? "0.95";
      try {
        const m = await get<Manifest>(`${base(this.getAttribute("cdn") ?? "")}/cdn/${encodeURIComponent(project)}/locales.json?min=${encodeURIComponent(min)}`);
        // `locales="es,de"`: only these (and the source) are offered.
        const only = (this.getAttribute("locales") ?? "").split(",").map((c) => c.trim()).filter(Boolean);
        this.#m = only.length ? { ...m, locales: m.locales.filter((l) => l.code === m.source || only.includes(l.code)) } : m;
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
      const variant = this.getAttribute("variant");
      const select = variant === "select";
      const labels = this.getAttribute("labels") ?? (select ? "name" : "code");
      const aria = this.getAttribute("aria-label") ?? "Language";
      const root = this.#root;
      this.#close();
      root.innerHTML = `<style>${CSS}</style>`;
      if (m.locales.length < 2) return;
      if (variant !== "pills" && !select) return this.#menu(m, current, labels, aria);
      if (select) {
        const s = el("select", { part: "select", "aria-label": aria }) as HTMLSelectElement;
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
      const list = el("div", { part: "list", role: "group", "aria-label": aria });
      for (const l of m.locales) {
        const on = l.code === current;
        const b = el("button", { type: "button", part: on ? "pill pill-current" : "pill", "aria-current": String(on), lang: l.code, dir: l.dir });
        b.append(el("span", {}, this.#label(l, labels)));
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

    // Default variant: a trigger (flag + code) opening a listbox of every language.
    #menu(m: Manifest, current: string, labels: string, aria: string) {
      const over: Record<string, string> = {};
      for (const a of this.attributes) if (a.name.startsWith("flag-")) over[a.name.slice(5)] = a.value;
      for (const k in this.#flags) over[k.toLowerCase()] = this.#flags[k];
      const flag = (l: Loc) => {
        const f = this.getAttribute("flags") === "off" ? "" : flagFor(l.code, over);
        return RI.test(f) && !flagsRender() ? "" : f;
      };
      const code = (l: Loc) => this.#label(l, "code");
      const cur = m.locales.find((l) => l.code === current)!;
      const shown = this.#label(cur, labels);
      const name = this.#label(cur, "name");
      const wrap = el("div", { class: "w" }); // the menu anchors here, not on a possibly stretched host
      const b = el("button", { type: "button", part: "trigger", "aria-haspopup": "listbox", "aria-expanded": "false", lang: cur.code, dir: cur.dir, "aria-label": `${aria}: ${name}${shown === name ? "" : ` (${shown})`}` });
      if (flag(cur)) b.append(el("span", { part: "flag", "aria-hidden": "true" }, flag(cur)));
      b.append(el("span", {}, shown));
      b.insertAdjacentHTML("beforeend", '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 4l3 3 3-3" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>');
      const pop = el("div", { part: "menu", hidden: "" });
      const ul = el("ul", { role: "listbox", "aria-label": aria });
      let names: Intl.DisplayNames | undefined;
      try {
        names = new Intl.DisplayNames([navigator.language], { type: "language" }); // "German" finds Deutsch
      } catch {}
      for (const l of m.locales) {
        const on = l.code === current;
        const o = el("li", { role: "option", "aria-selected": String(on), tabindex: "-1", lang: l.code, dir: l.dir, part: on ? "option option-current" : "option" });
        const f = flag(l);
        const n = this.#label(l, "name");
        o.dataset.q = fold(`${n} ${l.code} ${names?.of(l.code) ?? ""}`);
        o.append(f ? el("span", { part: "flag", "aria-hidden": "true" }, f) : el("span", { part: "flag", class: "b", "aria-hidden": "true" }, code(l)), el("span", {}, n));
        if (f || l.code.length > 2) o.append(el("small", {}, l.code));
        o.onclick = () => pick(l.code);
        ul.append(o);
      }
      const find = this.getAttribute("search-label") ?? "Search";
      const search = m.locales.length > 8 ? (el("input", { type: "search", part: "search", "aria-label": find, placeholder: find }) as HTMLInputElement) : null;
      if (search)
        search.oninput = () => {
          const q = fold(search.value.trim());
          for (const o of ul.children as HTMLCollectionOf<HTMLElement>) o.hidden = !o.dataset.q!.includes(q);
        };
      pop.append(...(search ? [search] : []), ul);
      wrap.append(b, pop);
      this.#root.append(wrap);

      const opts = () => [...(ul.children as HTMLCollectionOf<HTMLElement>)].filter((o) => !o.hidden);
      const outside = (e: Event) => e.composedPath().includes(this) || close();
      const close = (focus?: boolean) => {
        if (pop.hidden) return;
        pop.hidden = true;
        b.setAttribute("aria-expanded", "false");
        document.removeEventListener("pointerdown", outside, true);
        if (live === this) live = undefined;
        if (focus) b.focus();
      };
      // What had focus (index in [trigger, search, ...options]; -1 none). Removal fires focusout with no target: kept.
      const parts = () => [b, search, ...ul.children] as (HTMLElement | null)[];
      let f = -1;
      wrap.addEventListener("focusin", (e) => (f = parts().indexOf(e.target as HTMLElement)));
      wrap.addEventListener("focusout", (e) => e.relatedTarget && (f = -1));
      let at = -1;
      this.#close = (keep) => {
        if (keep && !pop.hidden) kept = { i: at, q: search?.value ?? "", f, t: Date.now() };
        close();
      };
      const open = (focus = true) => {
        pop.hidden = false;
        live = this;
        kept = undefined; // a menu opened since: the remembered one is stale
        at = nth(this);
        b.setAttribute("aria-expanded", "true");
        // Flip up when there is no room below (and more above).
        const r = b.getBoundingClientRect();
        const below = innerHeight - r.bottom;
        pop.classList.toggle("up", below < pop.offsetHeight + 8 && r.top > below);
        document.addEventListener("pointerdown", outside, true);
        const sel = ul.querySelector<HTMLElement>("[aria-selected=true]")!;
        if (focus) (search ?? sel).focus();
        sel.scrollIntoView?.({ block: "nearest" });
      };
      const pick = (c: string) => {
        close(true);
        this.#choose(c, current);
        this.#root.querySelector<HTMLElement>("[part~=trigger]")?.focus(); // re-rendered (event strategy)
      };
      b.onclick = () => (pop.hidden ? open() : close(true));
      let typed = "";
      let timer: ReturnType<typeof setTimeout>;
      wrap.onkeydown = (e) => {
        const k = e.key;
        const t = this.#root.activeElement as HTMLElement;
        if (pop.hidden) {
          // Enter / Space click the button natively.
          if (t === b && (k === "ArrowDown" || k === "ArrowUp")) e.preventDefault(), open();
          return;
        }
        if (k === "Escape") return e.preventDefault(), close(true);
        if (k === "Tab") return close();
        const os = opts();
        const i = os.indexOf(t);
        let to = -2;
        if (k === "ArrowDown") to = Math.min(i + 1, os.length - 1);
        else if (k === "ArrowUp") to = i > 0 ? i - 1 : search ? -1 : 0;
        else if (i >= 0 && k === "Home") to = 0;
        else if (i >= 0 && k === "End") to = os.length - 1;
        else if (k === "Enter" || (i >= 0 && k === " ")) {
          e.preventDefault();
          const o = i >= 0 ? os[i] : os[0];
          return o && pick(o.lang);
        } else if (i >= 0 && k.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          // Type-ahead: the next language whose name starts with what was typed.
          clearTimeout(timer);
          timer = setTimeout(() => (typed = ""), 500);
          typed += fold(k);
          const start = typed.length > 1 ? i : i + 1;
          const j = os.findIndex((_, j) => os[(start + j) % os.length].dataset.q!.startsWith(typed));
          if (j >= 0) to = (start + j) % os.length;
        }
        if (to === -2) return;
        e.preventDefault();
        (to === -1 ? search : os[to])?.focus();
      };

      const k = kept;
      if (k && Date.now() - k.t < 2000 && k.i === nth(this) && !live?.isConnected) {
        kept = undefined;
        if (search) (search.value = k.q), search.dispatchEvent(new Event("input"));
        open(false);
        const a = document.activeElement;
        if (k.f >= 0 && (!a || a === document.body)) parts()[k.f]?.focus();
      }
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
