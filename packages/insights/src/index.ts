// @glot/insights — anonymous locale demand + conversion beacon. No cookie, no visitor id:
// one sessionStorage entry holds the experiment arm, the server stores daily counters only.
export type Options = {
  /** Project public key (dashboard → Insights). */
  key: string;
  /** glot server origin. */
  endpoint?: string;
  /** Locale the app rendered (default: <html lang>). */
  locale?: string;
  /** Called when this session is in the test arm of a language experiment: render that locale. */
  onLocale?: (locale: string) => void;
};
type State = { a?: string; l?: string };

const SK = "glot:insights";
let opts: Options | undefined;

const read = (): State | null => {
  try {
    return JSON.parse(sessionStorage.getItem(SK) || "null");
  } catch {
    return null;
  }
};

function send(event?: string): Promise<State> {
  if (!opts) return Promise.resolve({});
  const body = {
    langs: [...(navigator.languages || [navigator.language])].slice(0, 5),
    served: opts.locale || document.documentElement.lang || "en",
    route: location.pathname,
    event,
    arm: read()?.a,
  };
  // String body = text/plain: a "simple" CORS request, no preflight.
  return fetch(`${opts.endpoint || ""}/api/insights/${opts.key}`, { method: "POST", body: JSON.stringify(body), keepalive: true })
    .then((r) => (r.ok ? r.json() : {}))
    .then((r: { arm?: string; locale?: string }) => ({ a: r.arm, l: r.locale }))
    .catch(() => ({}));
}

/** Counts one visit per browser session and applies a running language experiment. */
export async function init(o: Options): Promise<void> {
  opts = o;
  let s = read();
  if (!s) {
    s = await send();
    try {
      sessionStorage.setItem(SK, JSON.stringify(s));
    } catch {}
  }
  if (s.l) o.onLocale?.(s.l);
}

/** Conversion event: "signup", "purchase" or any name (1-40 chars of a-z0-9_-.:). */
export function track(event: string): void {
  void send(event);
}

export const glot = { init, track };

// <script src="insights.global.js" data-key="pk_…" data-endpoint="https://glot.example.com"> auto-inits.
const el = typeof document !== "undefined" ? (document.currentScript as HTMLScriptElement | null) : null;
if (el?.dataset.key) {
  (window as unknown as { glot: typeof glot }).glot = glot;
  void init({ key: el.dataset.key, endpoint: el.dataset.endpoint, locale: el.dataset.locale });
}
