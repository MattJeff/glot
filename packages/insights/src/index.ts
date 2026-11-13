// @glot/insights — anonymous locale demand + conversion beacon. No cookie, no visitor id:
// one sessionStorage entry holds the experiment arms, the server stores daily counters only.
export type Options = {
  /** Project public key (dashboard → Insights). */
  key: string;
  /** glot server origin. */
  endpoint?: string;
  /** Locale the app rendered (default: <html lang>). */
  locale?: string;
  /** Called when a language experiment or launch holdout assigns this session a locale (the new one, or the
   * fallback for held-out visitors): render that locale. Also dispatched as `glot:locale` on window. */
  onLocale?: (locale: string) => void;
  /** The visitor picked this locale (switcher, `?lang=`, account setting): never held out, excluded from launch
   * measurement. A pick in `<glot-locale-switcher>` (localStorage `glot-locale`) counts without this flag. */
  explicit?: boolean;
};
/** Copy variant served to this session for a key: experiment id, arm, text. */
export type Variant = { id: string; arm: number; value: string };
type State = { a?: string; l?: string; v?: Record<string, Variant> };

const SK = "glot:insights";
let opts: Options | undefined;

const read = (): State | null => {
  try {
    return JSON.parse(sessionStorage.getItem(SK) || "null");
  } catch {
    return null;
  }
};

/** Segmented-experiment assignment (contract §6): set by the edge, copied by @glot/runtime into sessionStorage. */
export type Exposure = { id: string; cell: string; arm: number };
export type Revenue = { value?: number; currency?: string };

/** An explicit language choice wins over any assignment (and leaves the launch analysis). */
const chosen = (): boolean => {
  if (opts?.explicit) return true;
  try {
    return !!localStorage.getItem("glot-locale");
  } catch {
    return false;
  }
};
const save = (s: State) => {
  try {
    sessionStorage.setItem(SK, JSON.stringify(s));
  } catch {}
};
const post = (body: object) =>
  fetch(`${opts!.endpoint || ""}/api/insights/${opts!.key}`, { method: "POST", body: JSON.stringify(body), keepalive: true });
const langs = () => [...(navigator.languages || [navigator.language])].slice(0, 5);
const served = () => opts!.locale || document.documentElement.lang || "en";

function send(event?: string, rev?: Revenue): Promise<State> {
  if (!opts) return Promise.resolve({});
  const explicit = chosen();
  const body = {
    langs: langs(),
    served: served(),
    route: location.pathname,
    event,
    arm: explicit ? undefined : read()?.a,
    explicit: explicit || undefined,
    variants: event ? arms() : undefined,
    exposures: event ? exposures(event) : undefined,
    value: rev?.value,
    currency: rev?.currency?.toUpperCase(),
  };
  // String body = text/plain: a "simple" CORS request, no preflight.
  return post(body)
    .then((r) => (r.ok ? r.json() : {}))
    .then((r: { arm?: string; locale?: string; variants?: (Variant & { key: string })[] }) => ({
      a: r.arm,
      l: r.locale,
      v: r.variants?.length ? Object.fromEntries(r.variants.map(({ key, ...v }) => [key, v])) : undefined,
    }))
    .catch(() => ({}));
}

/** Counts one visit per browser session and applies a running language experiment. */
export async function init(o: Options): Promise<void> {
  opts = o;
  let s = read();
  if (!s) {
    s = await send();
    save(s);
    // @glot/runtime's useGlotVariant / <GlotCopy> listen for this (no import between the packages).
    if (typeof dispatchEvent === "function") dispatchEvent(new Event("glot:variants"));
  }
  if (s.l && !chosen()) {
    o.onLocale?.(s.l);
    if (typeof dispatchEvent === "function") dispatchEvent(new CustomEvent("glot:locale", { detail: { locale: s.l, arm: s.a } }));
  }
}

/** The visitor picked a language mid-session (`glot:locale-change` from the switcher, or dispatched by your own
 * picker): the session leaves its language-experiment arm, once; the choice is kept by the switcher. */
function withdraw(): void {
  const s = read();
  if (!opts || !s?.a) return;
  void post({ langs: langs(), served: served(), arm: s.a, explicit: true }).catch(() => {});
  save({ v: s.v });
}
if (typeof addEventListener === "function") addEventListener("glot:locale-change", withdraw);

/** Conversion event: "signup", "purchase" or any name (1-40 chars of a-z0-9_-.:). Credits this session's
 * language-experiment arm, copy-variant arms and segmented-experiment exposures (each experiment once per
 * session and event). `value` (0..1e6) + `currency` (ISO 4217) count as revenue. */
export function track(event: string, rev?: Revenue): void {
  void send(event, rev);
}

const XK = "glot_xp", SENT = "glot_xp_sent";
const json = (k: string) => {
  try {
    return JSON.parse(sessionStorage.getItem(k) || "null");
  } catch {
    return null;
  }
};

const noTrack = () => {
  const nav = navigator as Navigator & { globalPrivacyControl?: boolean };
  // Automated browsers (Playwright, Puppeteer, Selenium, headless QA) are not visitors.
  return nav.globalPrivacyControl || nav.doNotTrack === "1" || nav.webdriver === true;
};

/** Marks `id:event` credited this session; false when it already was. */
function once(key: string): boolean {
  const sent: string[] = json(SENT) ?? [];
  if (sent.includes(key)) return false;
  try {
    sessionStorage.setItem(SENT, JSON.stringify([...sent, key]));
  } catch {}
  return true;
}

/** "Seen" exposure from @glot/runtime (`glot:seen`): posted to the experiments batch, once per session/experiment. */
function seen(e: Event): void {
  const x = (e as CustomEvent<Exposure & { seen: true; visible_ms: number }>).detail;
  if (!opts || !x || noTrack() || !once(`${x.id}:$seen`)) return;
  const body = [{ served: opts.locale || document.documentElement.lang || "en", exposures: [x] }];
  void fetch(`${opts.endpoint || ""}/api/insights/${opts.key}/batch`, { method: "POST", body: JSON.stringify(body), keepalive: true }).catch(() => {});
}
if (typeof addEventListener === "function") addEventListener("glot:seen", seen);

/** Exposures to credit for `event`: cookie `glot_xp` (persistence "session") + sessionStorage (SSR copy), minus
 * those already credited for this event; none under Global Privacy Control / Do Not Track. */
function exposures(event: string): Exposure[] | undefined {
  if (noTrack()) return;
  const all = new Map<string, Exposure>();
  const cookie = typeof document !== "undefined" ? /(?:^|;\s*)glot_xp=([^;]*)/.exec(document.cookie || "")?.[1] : undefined;
  for (const p of cookie?.split("~") ?? []) {
    const [id, arm, cell] = p.split(".");
    if (id && cell && arm) all.set(id, { id, cell, arm: Number(arm) });
  }
  for (const e of (json(XK) as Exposure[] | null) ?? []) all.set(e.id, e);
  const out = [...all.values()].slice(0, 20).filter((e) => once(`${e.id}:${event}`));
  return out.length ? out : undefined;
}

function arms(): Record<string, number> | undefined {
  const v = read()?.v;
  return v ? Object.fromEntries(Object.values(v).map((x) => [x.id, x.arm])) : undefined;
}

/** Copy variant text served to this session for `key` (a validated variant of a running experiment), if any. */
export function variant(key: string): string | undefined {
  return read()?.v?.[key]?.value;
}

export const glot = { init, track, variant };

// <script src="insights.global.js" data-key="pk_…" data-endpoint="https://glot.example.com"> auto-inits.
const el = typeof document !== "undefined" ? (document.currentScript as HTMLScriptElement | null) : null;
if (el?.dataset.key) {
  (window as unknown as { glot: typeof glot }).glot = glot;
  void init({ key: el.dataset.key, endpoint: el.dataset.endpoint, locale: el.dataset.locale, explicit: !!el.dataset.explicit });
}
