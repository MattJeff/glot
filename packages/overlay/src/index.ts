// @glot/overlay — edit translations in your running app. Alt+click any text (Alt+Shift+G = inspect mode).
// Talks to `glot dev` (localhost). Dev only: does nothing when NODE_ENV === "production".
import { CSS } from "./style";

export interface OverlayOptions {
  /** glot dev URL. Default: origin of the <script src> serving it, else http://127.0.0.1:4317 */
  url?: string;
  /** Token printed by `glot dev`. Else ?glot_token=, data-token, localStorage, or asked in the UI. */
  token?: string;
  /** Cookie used to switch the app locale (next-intl default). */
  localeCookie?: string;
}

type Issue = { code: string; severity: "error" | "warning"; message: string };
type Match = { key: string; locale: string; value: string; score: number };
type KeyInfo = {
  key: string;
  source_locale: string;
  values: Record<string, string | null>;
  issues: Record<string, Issue[]>;
  note?: string | null;
  refs: { path: string; line: number }[];
  component?: string | null;
};

const LS = "glot-dev-token";
const store = {
  get: () => { try { return localStorage.getItem(LS) || ""; } catch { return ""; } },
  set: (t: string) => { try { localStorage.setItem(LS, t); } catch {} },
};

// Tiny hyperscript.
function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, any> = {},
  ...kids: (Node | string | null | false | undefined)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k in el && k !== "list") (el as any)[k] = v;
    else el.setAttribute(k, String(v));
  }
  for (const c of kids) if (c != null && c !== false) el.append(c);
  return el;
}

let started = false;

export function init(opts: OverlayOptions = {}): void {
  if (started || typeof window === "undefined") return;
  started = true;

  const script = document.currentScript as HTMLScriptElement | null;
  const url = (opts.url || script?.dataset.url || (script?.src ? new URL(script.src).origin : "") || "http://127.0.0.1:4317").replace(/\/$/, "");
  const params = new URLSearchParams(location.search);
  let token = opts.token || params.get("glot_token") || script?.dataset.token || store.get();
  if (params.get("glot_token")) store.set(token);
  const cookie = opts.localeCookie || "NEXT_LOCALE";

  // ---- shell ----
  const host = h("glot-overlay" as any, { style: "all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647" });
  const root = host.attachShadow({ mode: "open" });
  root.append(h("style", {}, CSS));
  const hl = h("div", { className: "hl" });
  const pop = h("div", { className: "pop", role: "dialog", "aria-label": "glot" });
  const pill = h("div", { className: "pill" }, "glot · inspect — click any text, Esc to quit");
  root.append(hl, pop, pill);
  document.documentElement.append(host);

  // ---- api ----
  async function api(path: string, body?: unknown): Promise<any> {
    const r = await fetch(url + path, {
      method: body === undefined ? "GET" : "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const j = await r.json().catch(() => ({}));
    if (r.status === 401) throw Object.assign(new Error("unauthorized"), { auth: true });
    if (!r.ok) throw new Error(j?.error?.message || r.statusText);
    return j;
  }

  // ---- state ----
  let inspect = false;
  let target: HTMLElement | null = null;
  let current: { matches: Match[]; key?: string; text: string } | null = null;
  let es: EventSource | null = null;
  let locales: string[] = [];
  let lastSig = "";
  let flash = ""; // message surviving one re-render (e.g. after "Translate all")

  function listen() {
    es?.close();
    if (!token) return;
    es = new EventSource(`${url}/api/events?token=${encodeURIComponent(token)}`);
    es.addEventListener("change", () => { if (current?.key && pop.classList.contains("open")) showKey(current.key, true); });
  }
  listen();

  // ---- geometry ----
  function place() {
    if (!target) return;
    const r = target.getBoundingClientRect();
    const w = Math.min(400, innerWidth - 16);
    const left = Math.max(8, Math.min(r.left, innerWidth - w - 8));
    pop.style.width = w + "px";
    pop.style.left = left + "px";
    const below = innerHeight - r.bottom, above = r.top;
    if (below >= 320 || below >= above) {
      pop.style.top = r.bottom + 8 + "px"; pop.style.bottom = "";
      pop.style.maxHeight = Math.max(200, below - 16) + "px";
    } else {
      pop.style.bottom = innerHeight - r.top + 8 + "px"; pop.style.top = "";
      pop.style.maxHeight = Math.max(200, above - 16) + "px";
    }
  }
  function highlight(el: Element | null, kind = "") {
    if (!el) { hl.style.display = "none"; return; }
    const r = el.getBoundingClientRect();
    Object.assign(hl.style, { display: "block", left: r.left - 3 + "px", top: r.top - 3 + "px", width: r.width + 6 + "px", height: r.height + 6 + "px" });
    hl.className = "hl " + kind;
  }

  /** Does the element clip / spill out of itself or its parent box? */
  function overflow(el: HTMLElement): string | null {
    const cs = getComputedStyle(el);
    const clipX = cs.overflowX !== "visible" && el.scrollWidth > el.clientWidth + 2;
    const clipY = cs.overflowY !== "visible" && el.scrollHeight > el.clientHeight + 2;
    if (clipX || clipY) return "text is clipped (ellipsis / hidden overflow / line clamp)";
    const p = el.parentElement;
    if (p) {
      const a = el.getBoundingClientRect(), b = p.getBoundingClientRect();
      if (a.right > b.right + 2 || a.left < b.left - 2) return "element spills out of its container";
    }
    return null;
  }

  // ---- popover ----
  function open(nodes: (Node | string)[]) {
    pop.replaceChildren(...nodes);
    pop.classList.add("open");
    place();
  }
  function close() {
    pop.classList.remove("open");
    highlight(null);
    target = null; current = null;
  }
  const head = (title: Node | string, ...extra: Node[]) =>
    h("div", { className: "head" }, h("div", { className: "title" }, title), ...extra,
      h("button", { className: "icon", title: "Close (Esc)", onclick: close }, "×"));

  function askToken(retry: () => void) {
    const input = h("input", { className: "in", placeholder: "token printed by `glot dev`", value: token });
    const go = () => { token = input.value.trim(); store.set(token); listen(); retry(); };
    open([
      head("Connect to glot dev"),
      h("p", { className: "muted" }, "Run ", h("code", {}, "glot dev"), " in your project and paste the token."),
      h("div", { className: "row" }, input, h("button", { className: "btn primary", onclick: go }, "Connect")),
    ]);
    input.addEventListener("keydown", (e) => e.key === "Enter" && go());
    input.focus();
  }

  function fail(e: any, retry: () => void) {
    if (e?.auth) return askToken(retry);
    open([head("glot"), h("p", { className: "err" }, e?.message === "Failed to fetch" ? `glot dev is not reachable at ${url}` : String(e?.message || e))]);
  }

  async function pick(el: HTMLElement) {
    target = el;
    highlight(el, "sel");
    open([head("Looking up…")]);
    // Innermost text first, then up to two ancestors while inline (rich text: clicked <b> inside a message).
    let text = "", matches: Match[] = [];
    try {
      for (let n: HTMLElement | null = el, i = 0; n && i < 3 && !matches.length; n = getComputedStyle(n).display === "inline" ? n.parentElement : null, i++) {
        const t = (n.innerText || n.textContent || "").trim();
        if (!t || t.length > 500) break;
        if (!text) text = t;
        matches = (await api("/api/lookup", { text: t })).matches;
      }
    } catch (e) { return fail(e, () => pick(el)); }
    current = { matches, text };
    if (!matches.length) return showMissing(text);
    showKey(matches[0].key);
  }

  async function showKey(key: string, quiet = false) {
    if (!current) return;
    current.key = key;
    let info: KeyInfo;
    try { info = await api(`/api/key?key=${encodeURIComponent(key)}`); } catch (e) { return fail(e, () => showKey(key)); }
    const sig = JSON.stringify(info);
    if (quiet && (sig === lastSig || root.activeElement?.tagName === "TEXTAREA")) return; // unchanged, or user typing
    lastSig = sig;
    locales = Object.keys(info.values);
    const shown = current.matches.find((m) => m.key === key)?.locale || info.source_locale;
    let active = shown;
    const ov = target && overflow(target);
    const chips = current.matches.length > 1
      ? h("div", { className: "chips" }, ...current.matches.map((m) =>
          h("button", { className: "chip" + (m.key === key ? " on" : ""), onclick: () => showKey(m.key) }, m.key)))
      : null;
    const meta = h("div", { className: "meta" },
      ov ? h("span", { className: "badge warn", title: ov }, "⚠ overflow") : h("span", { className: "badge ok", title: "fits its container" }, "fits"),
      info.component ? h("span", {}, `<${info.component}>`) : null,
      ...info.refs.slice(0, 2).map((r) => h("span", { className: "mono" }, `${r.path}:${r.line}`)),
    );
    const sugg = h("div", { className: "sugg" }, flash ? h("div", { className: "okmsg" }, flash) : "");
    flash = "";
    const rows: Record<string, HTMLTextAreaElement> = {};

    const rowFor = (loc: string) => {
      const v = info.values[loc];
      const ta = h("textarea", { className: "in", rows: 1, value: v ?? "", placeholder: "missing", spellcheck: false, dir: "auto" });
      rows[loc] = ta;
      const status = h("span", { className: "status" });
      const issues = h("div", { className: "issues" }, ...(info.issues[loc] || []).map(badge));
      const grow = () => { ta.style.height = "auto"; ta.style.height = ta.scrollHeight + "px"; };
      requestAnimationFrame(grow);
      let saved = v ?? "";
      const save = async () => {
        if (ta.value === saved) return;
        status.textContent = "saving…"; status.className = "status";
        try {
          const r = await api("/api/set", { key, locale: loc, value: ta.value });
          issues.replaceChildren(...r.issues.map(badge));
          if (r.ok) { saved = ta.value; status.textContent = "✓ saved"; status.className = "status ok"; }
          else { status.textContent = "rejected"; status.className = "status bad"; }
        } catch (e: any) { status.textContent = e.message; status.className = "status bad"; }
      };
      ta.addEventListener("input", grow);
      ta.addEventListener("focus", () => { active = loc; aiLabel.textContent = loc; });
      ta.addEventListener("blur", save);
      ta.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); save(); }
        if (e.key === "Escape") { ta.value = saved; ta.blur(); }
      });
      return h("div", { className: "loc" + (loc === shown ? " shown" : "") },
        h("div", { className: "lochead" },
          h("span", { className: "code" }, loc), loc === info.source_locale ? h("span", { className: "tag" }, "source") : null,
          loc === shown ? h("span", { className: "tag" }, "on screen") : null,
          status,
          loc !== shown ? h("button", { className: "link", title: `Reload the app in ${loc}`, onclick: () => viewIn(loc) }, "view") : null),
        ta, issues);
    };

    const ai = async (instruction: string) => {
      sugg.replaceChildren(h("div", { className: "muted" }, "Thinking…"));
      try {
        const r = await api("/api/suggest", { key, locale: active, instruction });
        sugg.replaceChildren(...(r.suggestions.length ? r.suggestions.map((s: { value: string }) =>
          h("button", { className: "suggestion", title: "Use this", onclick: () => {
            const ta = rows[active]; ta.value = s.value; ta.dispatchEvent(new Event("input")); ta.focus(); ta.blur();
            sugg.replaceChildren();
          } }, s.value)) : [h("div", { className: "muted" }, "No valid suggestion.")]));
        sugg.scrollIntoView({ block: "nearest" });
      } catch (e: any) { sugg.replaceChildren(h("div", { className: "err" }, e.message)); }
    };
    const translateAll = async () => {
      sugg.replaceChildren(h("div", { className: "muted" }, "Translating…"));
      try {
        const r = await api("/api/translate", { key });
        const n = Object.keys(r.written || {}).length, bad = Object.keys(r.rejected || {}).length;
        flash = (n ? `Translated into ${n} locale(s)${bad ? `, ${bad} rejected` : ""}.` : "Nothing missing or stale.");
        showKey(key);
      } catch (e: any) { sugg.replaceChildren(h("div", { className: "err" }, e.message)); }
    };
    const aiLabel = h("span", { className: "code" }, active);
    const order = [...locales].sort((a, b) => (a === shown ? -1 : b === shown ? 1 : a === info.source_locale ? -1 : b === info.source_locale ? 1 : a.localeCompare(b)));

    open([
      head(h("span", { className: "mono" }, key)),
      chips || "",
      meta,
      info.note ? h("p", { className: "note" }, info.note) : "",
      rowFor(order[0]),
      h("div", { className: "ai" },
        h("span", { className: "muted" }, "AI ", aiLabel),
        h("button", { className: "btn", onclick: () => ai("variants") }, "Suggest"),
        h("button", { className: "btn", onclick: () => ai("shorten") }, "Shorten"),
        h("button", { className: "btn", onclick: () => ai("formal") }, "More formal"),
        h("button", { className: "btn primary", onclick: translateAll }, "Translate all")),
      sugg,
      h("div", { className: "locs" }, ...order.slice(1).map(rowFor)),
    ]);
  }

  async function showMissing(text: string) {
    let p: any;
    try { p = await api("/api/propose", { text }); } catch (e) { return fail(e, () => showMissing(text)); }
    const keyIn = h("input", { className: "in mono", value: p.key });
    const out = h("div", {});
    const go = async () => {
      try {
        const r = await api("/api/extract", { key: keyIn.value, value: text });
        out.replaceChildren(h("p", { className: "okmsg" }, "✓ Added ", h("code", {}, r.key), ". Replace the literal in code with ", h("code", {}, `t("${r.key}")`), " (or run ", h("code", {}, "glot i18nize"), ")."));
      } catch (e: any) { out.replaceChildren(h("p", { className: "err" }, e.message)); }
    };
    open([
      head("Hardcoded text"),
      h("p", { className: "muted" }, "No translation key renders this text:"),
      h("blockquote", {}, text.length > 160 ? text.slice(0, 160) + "…" : text),
      ...(p.refs || []).slice(0, 3).map((r: any) => h("div", { className: "mono muted" }, `${r.path}:${r.line}`)),
      ...p.duplicates.map(([k]: [string]) => h("div", { className: "badge warn" }, `same text exists as ${k}`)),
      h("div", { className: "row" }, keyIn, h("button", { className: "btn primary", onclick: go }, "Extract to key")),
      out,
    ]);
    keyIn.addEventListener("keydown", (e) => e.key === "Enter" && go());
  }

  function viewIn(loc: string) {
    document.cookie = `${cookie}=${loc}; path=/; max-age=31536000; samesite=lax`;
    // Locale-prefixed routes (/fr/...): swap the first segment when it is a known locale.
    const seg = location.pathname.split("/");
    if (seg[1] && locales.includes(seg[1])) { seg[1] = loc; location.pathname = seg.join("/"); }
    else location.reload();
  }

  function badge(i: Issue) {
    return h("span", { className: "badge " + (i.severity === "error" ? "bad" : "warn"), title: i.message }, i.code.replace(/_/g, " "));
  }

  // ---- input ----
  const ours = (e: Event) => e.composedPath().includes(host);
  const setInspect = (on: boolean) => {
    inspect = on;
    pill.classList.toggle("open", on);
    if (!on && !pop.classList.contains("open")) highlight(null);
  };
  addEventListener("click", (e) => {
    if (ours(e)) return;
    if (e.altKey || inspect) {
      const el = e.target as HTMLElement;
      if (!(el instanceof HTMLElement)) return;
      e.preventDefault(); e.stopPropagation();
      setInspect(false);
      pick(el);
    } else if (pop.classList.contains("open")) close();
  }, true);
  addEventListener("mouseover", (e) => {
    if (!inspect || ours(e)) return;
    highlight(e.target as Element, "hover");
  }, true);
  addEventListener("keydown", (e) => {
    if (e.altKey && e.shiftKey && e.code === "KeyG") { e.preventDefault(); setInspect(!inspect); }
    else if (e.key === "Escape") { if (inspect) setInspect(false); else if (pop.classList.contains("open") && !ours(e)) close(); }
  }, true);
  root.addEventListener("keydown", (e: KeyboardEvent) => { if ((e as KeyboardEvent).key === "Escape" && (e.target as HTMLElement).tagName !== "TEXTAREA") close(); });
  addEventListener("scroll", () => { if (target) { place(); highlight(target, "sel"); } }, true);
  addEventListener("resize", () => { if (target) { place(); highlight(target, "sel"); } });
}

// Auto-start on import / <script>, in dev only.
declare const process: { env: Record<string, string | undefined> } | undefined;
const prod = typeof process !== "undefined" && process.env?.NODE_ENV === "production";
if (typeof window !== "undefined" && !prod && !(window as any).GLOT_OVERLAY_MANUAL) {
  const s = document.currentScript as HTMLScriptElement | null;
  const go = () => init();
  // Keep currentScript readable: init synchronously when loaded as a classic script.
  if (s) init(); else if (document.readyState === "loading") addEventListener("DOMContentLoaded", go); else go();
}
