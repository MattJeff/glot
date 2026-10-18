// @glot/runtime — on-device fallback translation (Chrome Translator API) with glot's guarantees:
// placeholders/tags/glossary masked, plural structure kept, every result validated by glot-core (WASM);
// anything that fails stays in the source language.
import { showBadge, type BadgeOptions } from "./badge";
import { mask, terms, i18nextPluralKeys, type Glossary, type Syntax, type Unit } from "./mask";

export { mask, parseIcu, i18nextPluralKeys, type Glossary, type Term, type Syntax } from "./mask";

export { showBadge, type BadgeOptions };
export const VERSION = "0.1.0";

type Json = { [k: string]: string | Json | unknown };

export type Status =
  | "translated"            // messages are the on-device translation (possibly partial: see `fellBack`)
  | "source"                // locale === sourceLocale
  | "unsupported"           // no Translator API (Firefox, Safari, mobile, http://, worker)
  | "unavailable"           // the browser can't translate this language pair
  | "needs-activation"      // model must be downloaded: call `prepareTranslator` from a click, then retry
  | "validator-unavailable" // glot WASM failed to load: nothing unvalidated is ever shown
  | "error";

export interface FallbackResult {
  messages: Json;
  status: Status;
  locale: string;
  /** Keys (dotted) that failed masking/translation/validation and stayed in the source language. */
  fellBack: string[];
  translated: number;
  cached: number;
}

export interface FallbackOptions {
  /** Source catalog (nested JSON, e.g. `messages/en.json`). */
  source: Json;
  locale: string;
  sourceLocale?: string;
  glossary?: Glossary;
  syntax?: Syntax;
  /** Model download progress, 0..1. */
  onProgress?: (loaded: number) => void;
  /** Where `glot.wasm` lives (default: next to this module). Or the bytes. */
  wasm?: string | URL | BufferSource;
  /** IndexedDB cache (default on when IndexedDB exists). */
  cache?: boolean;
  /** Cache key part; the Translator API exposes no model version, so default = browser build + runtime version. */
  modelVersion?: string;
  /** Max characters per translate() call (segments joined by newlines). */
  batchChars?: number;
}

// ---------------------------------------------------------------- Translator API

interface TranslatorLike { translate(text: string): Promise<string> }
const api = (): any => (globalThis as any).Translator;
const lang = (l: string) => l.replace("_", "-");
const translators = new Map<string, Promise<TranslatorLike>>();

export const isSupported = (): boolean => typeof api()?.create === "function";

/** Availability of the pair: "unavailable" | "downloadable" | "downloading" | "available" | "unsupported". */
export async function availability(sourceLanguage: string, targetLanguage: string): Promise<string> {
  if (!isSupported()) return "unsupported";
  try {
    // ponytail: 5 s cap — some builds (headless Chromium) never settle; treat as unavailable.
    return await Promise.race([
      api().availability({ sourceLanguage: lang(sourceLanguage), targetLanguage: lang(targetLanguage) }),
      new Promise<string>((ok) => setTimeout(ok, 5000, "unavailable")),
    ]);
  } catch {
    return "unavailable";
  }
}

/** Create (and download if needed) the translator. Needs a user activation when the model isn't downloaded yet. */
export function prepareTranslator(sourceLanguage: string, targetLanguage: string, onProgress?: (loaded: number) => void): Promise<TranslatorLike> {
  const key = `${sourceLanguage}>${targetLanguage}`;
  let t = translators.get(key);
  if (!t) {
    t = api().create({
      sourceLanguage: lang(sourceLanguage),
      targetLanguage: lang(targetLanguage),
      monitor(m: EventTarget) {
        m.addEventListener("downloadprogress", (e: any) => onProgress?.(e.loaded));
      },
    }) as Promise<TranslatorLike>;
    t.catch(() => translators.delete(key));
    translators.set(key, t);
  }
  return t;
}

// ---------------------------------------------------------------- validator (glot-core → WASM)

export interface Issue { code: string; severity: "error" | "warning"; message: string }
export type Validate = (input: { source: string; target: string; locale: string; syntax?: Syntax; glossary?: unknown[] }) => Issue[];

let validator: Promise<Validate> | undefined;

/** Load glot's validator (crates/wasm). Cached after the first call. */
export function loadValidator(src: string | URL | BufferSource = new URL("./glot.wasm", import.meta.url)): Promise<Validate> {
  return (validator ??= (async (): Promise<Validate> => {
    const bytes = src instanceof URL || typeof src === "string" ? await (await fetch(src)).arrayBuffer() : src;
    const { instance } = await WebAssembly.instantiate(bytes, {});
    const x = instance.exports as any;
    const enc = new TextEncoder(), dec = new TextDecoder();
    return (input) => {
      const data = enc.encode(JSON.stringify(input));
      const p = x.alloc(data.length);
      new Uint8Array(x.memory.buffer, p, data.length).set(data);
      const r = x.check(p, data.length);
      x.dealloc(p, data.length);
      const len = new DataView(x.memory.buffer).getUint32(r, true);
      const out = dec.decode(new Uint8Array(x.memory.buffer, r + 4, len));
      x.dealloc(r, len + 4);
      return JSON.parse(out);
    };
  })().catch((e) => { validator = undefined; throw e; }));
}

// ---------------------------------------------------------------- IndexedDB cache

const DB = "glot-runtime";
function idb(): Promise<IDBDatabase> | null {
  if (typeof indexedDB === "undefined") return null;
  return new Promise((ok, ko) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore("t");
    r.onsuccess = () => ok(r.result);
    r.onerror = () => ko(r.error);
  });
}
const req = <T>(r: IDBRequest<T>) => new Promise<T>((ok, ko) => { r.onsuccess = () => ok(r.result); r.onerror = () => ko(r.error); });

async function sha(s: string): Promise<string> {
  const d = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s));
  return [...new Uint8Array(d).slice(0, 12)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

const defaultModelVersion = () =>
  `${/(?:Chrome|Chromium|Edg)\/(\d+)/.exec(globalThis.navigator?.userAgent ?? "")?.[1] ?? "?"}+${VERSION}`;

// ---------------------------------------------------------------- main

function flatten(o: Json, pre = "", out: Record<string, string> = {}): Record<string, string> {
  for (const [k, v] of Object.entries(o)) {
    const key = pre ? `${pre}.${k}` : k;
    if (typeof v === "string") out[key] = v;
    else if (v && typeof v === "object") flatten(v as Json, key, out);
  }
  return out;
}

function unflatten(base: Json, flat: Record<string, string>): Json {
  const out = structuredClone(base);
  for (const [k, v] of Object.entries(flat)) {
    const path = k.split(".");
    let o: any = out;
    for (const p of path.slice(0, -1)) o = o[p] ??= {};
    o[path.at(-1)!] = v;
  }
  return out;
}

/**
 * Translate `source` into `locale` on the device. Never throws: on any problem the result is the source
 * catalog with a `status` explaining why. Safe to call on every page load (IndexedDB cache).
 */
export async function createFallbackMessages(opts: FallbackOptions): Promise<FallbackResult> {
  const { source, locale, sourceLocale = "en", syntax = "icu", glossary } = opts;
  const result = (status: Status, messages: Json = source, extra: Partial<FallbackResult> = {}): FallbackResult =>
    ({ messages, status, locale, fellBack: [], translated: 0, cached: 0, ...extra });
  if (lang(locale).toLowerCase() === lang(sourceLocale).toLowerCase()) return result("source");
  if (!isSupported()) return result("unsupported");

  const av = await availability(sourceLocale, locale);
  if (av === "unavailable" || av === "unsupported") return result("unavailable");
  const key = `${sourceLocale}>${locale}`;
  if (av !== "available" && !translators.has(key) && !(globalThis.navigator as any)?.userActivation?.isActive)
    return result("needs-activation");

  let check: Validate, translator: TranslatorLike;
  try {
    check = await loadValidator(opts.wasm);
  } catch {
    return result("validator-unavailable");
  }
  try {
    translator = await prepareTranslator(sourceLocale, locale, opts.onProgress);
  } catch {
    return result(av === "available" ? "error" : "needs-activation");
  }

  const flat = flatten(source);
  const masked = Object.fromEntries(Object.entries(flat).map(([k, v]) => [k, mask(v, { syntax, glossary, locale })]));
  const cores = [...new Set(Object.values(masked).flatMap((m) => m?.units.map((u) => u.core) ?? []).filter(Boolean))];

  // 1. cache
  const done = new Map<string, string>();
  const version = opts.modelVersion ?? defaultModelVersion();
  const cacheKey = async (c: string) => `${locale}|${version}|${await sha(c)}`;
  const db = opts.cache === false ? null : await idb()?.catch(() => null) ?? null;
  let cached = 0;
  const keys = new Map<string, string>();
  if (db) {
    for (const c of cores) keys.set(c, await cacheKey(c)); // before the transaction: awaits would close it
    const store = db.transaction("t").objectStore("t");
    await Promise.all(cores.map(async (c) => {
      const hit = await req(store.get(keys.get(c)!)).catch(() => undefined);
      if (typeof hit === "string") { done.set(c, hit); cached++; }
    }));
  }

  // 2. translate the rest, in newline-joined batches (the API processes calls sequentially anyway)
  const todo = cores.filter((c) => !done.has(c));
  const fresh = new Map<string, string>();
  const max = opts.batchChars ?? 1000;
  const one = async (c: string) => { try { fresh.set(c, await translator.translate(c)); } catch { /* stays source */ } };
  for (let i = 0; i < todo.length;) {
    const batch: string[] = [];
    let n = 0;
    while (i < todo.length && (batch.length === 0 || n + todo[i].length < max) && !todo[i].includes("\n")) {
      n += todo[i].length + 1;
      batch.push(todo[i++]);
    }
    if (batch.length === 0) { await one(todo[i++]); continue; }
    if (batch.length === 1) { await one(batch[0]); continue; }
    let lines: string[] = [];
    try { lines = (await translator.translate(batch.join("\n"))).split("\n"); } catch { /* retry one by one */ }
    if (lines.length === batch.length) batch.forEach((c, k) => fresh.set(c, lines[k]));
    else for (const c of batch) await one(c);
  }

  // 3. restore + validate; failures fall back to the source string
  const gl = terms(glossary);
  const out: Record<string, string> = {};
  const fellBack: string[] = [];
  const good = new Set<string>();
  const get = (u: Unit) => {
    const t = done.get(u.core) ?? fresh.get(u.core);
    if (t === undefined) throw 0;
    return t;
  };
  for (const [k, src] of Object.entries(flat)) {
    let target: string | null = null;
    try { target = masked[k]?.restore(get) ?? null; } catch { target = null; }
    const ok = target !== null &&
      !check({ source: src, target, locale, syntax, glossary: gl }).some((i) => i.severity === "error");
    if (ok) {
      out[k] = target!;
      masked[k]!.units.forEach((u) => u.core && good.add(u.core));
    } else {
      out[k] = src;
      fellBack.push(k);
    }
  }

  // 4. cache only segments that ended up in a validated message
  if (db) {
    const store = db.transaction("t", "readwrite").objectStore("t");
    for (const [c, t] of fresh) if (good.has(c)) store.put(t, keys.get(c)!);
  }

  const final = syntax === "i18next" ? i18nextPluralKeys(out, locale) : out;
  return result("translated", unflatten(source, final), {
    fellBack,
    translated: Object.keys(flat).length - fellBack.length,
    cached,
  });
}

// ---------------------------------------------------------------- locale helpers

/** First browser language that the app doesn't ship (and isn't the source), or null. */
export function wantedLocale(shipped: readonly string[], languages: readonly string[] = globalThis.navigator?.languages ?? []): string | null {
  const base = (l: string) => l.toLowerCase().split(/[-_]/)[0];
  const l = languages[0];
  return !l || shipped.some((s) => base(s) === base(l)) ? null : l;
}

// ---------------------------------------------------------------- badge + activation flow

export interface AutoOptions extends Omit<FallbackOptions, "onProgress"> {
  /** `false` hides the badge (and the click-to-download prompt). */
  badge?: false | Pick<BadgeOptions, "label" | "improveLabel" | "improveUrl" | "activateLabel">;
  /** Called with the translated catalog — right away, or later once the user allowed the model download. */
  onMessages?: (messages: Json, result: FallbackResult) => void;
}

/**
 * createFallbackMessages + badge: shows "translated on your device · Improve", the download percentage, and
 * when the model isn't downloaded yet a "Translate this page on your device" button (the API needs a click).
 * The badge texts are translated along with the catalog.
 */
export async function fallbackWithBadge(o: AutoOptions): Promise<FallbackResult> {
  const badge = o.badge === false ? null : o.badge ?? {};
  const source = badge
    ? { ...o.source, __glot_badge: { label: badge.label ?? "Automatically translated on your device", improve: badge.improveLabel ?? "Improve" } }
    : o.source;
  const progress = (p: number) => badge && showBadge({ ...badge, locale: o.locale, progress: p });
  const run = async (): Promise<FallbackResult> => {
    const r = await createFallbackMessages({ ...o, source, onProgress: progress });
    const { __glot_badge: b, ...messages } = r.messages as { __glot_badge?: { label: string; improve: string } };
    const res = { ...r, messages };
    if (r.status === "translated") {
      if (badge) showBadge({ ...badge, locale: o.locale, label: b?.label, improveLabel: b?.improve });
      o.onMessages?.(messages, res);
    } else if (r.status === "needs-activation" && badge) {
      const remove = showBadge({
        ...badge,
        onActivate: () => {
          // Synchronous call inside the click: create() needs the user activation.
          prepareTranslator(o.sourceLocale ?? "en", o.locale, progress).then(run, remove);
        },
      });
    }
    return res;
  };
  return run();
}

