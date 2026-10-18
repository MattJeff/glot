// Segmented copy experiments at the edge (contract: docs/design/segmented-copy-experiments-contract.md §5–7).
// `decide()` is pure and sync: segment → holdout → one arm per experiment, then the app gets `x-glot-xp`
// (assignments + overlay for the served locale) before it renders: no flicker, glot never on the critical path.
// The config comes from `GET /cdn/{slug}/experiments.json`, cached in memory, revalidated in the background;
// a cold start waits at most `timeout` ms, then serves the official text. No dependencies.

export type Rule = { all: { attr: string; eq?: string; in?: string[]; not_in?: string[] }[] };
export type XpConfig = {
  v: 1;
  generated_at: string;
  /** Set by the loader on each 200/304 (contract v1.3: `max_stale` counts from there, the ETag ignores `generated_at`). */
  fetched_at?: number;
  max_stale: number;
  persistence: "none" | "session";
  holdout: { id: string; pct: number } | null;
  segments: { id: string; rule: Rule }[];
  experiments: {
    id: string;
    status: string;
    traffic_pct: number;
    locales: string[];
    /** Keys under test (contract v1.6): forwarded as `x-glot-xp.k` so the runtime can measure "seen". */
    keys?: string[];
    cells: { id: string; weights: number[]; count: boolean }[];
    arms: { arm: number; values?: Record<string, Record<string, string>> }[];
  }[];
};
export type Assignment = { id: string; cell: string; arm: number };
export type Decision = {
  /** Every assignment of this document, holdout included (`x-glot-xp.a`). */
  assignments: Assignment[];
  /** key → text for the served locale (arm 0 never appears). */
  overlay: Record<string, string>;
  /** What to count: empty for bots, prefetches, opt-out, `?glot_xp=` previews, personalized cells. */
  exposures: Assignment[];
  /** `x-glot-xp` request header for the app, or null (= official messages). */
  header: string | null;
  /** `Set-Cookie` values to add to the response. */
  cookies: string[];
};
export type DecideOptions = {
  /** Locale being served (from `route()`). */
  locale: string;
  /** Consent signal from your CMP: only then is a persistent cookie used (`visitor` = new/returning). */
  consent?: boolean;
  /** App attributes (`plan`, `logged_in`, `custom.*`): matched here, never sent to glot. */
  attrs?: Record<string, string>;
  /** Stable unit to hash (an id you already have): same unit ⇒ same arms. Default: random per document. */
  unit?: string;
  salt?: string;
  now?: number;
  rand?: () => number;
};

const NONE: Decision = { assignments: [], overlay: {}, exposures: [], header: null, cookies: [] };
const BOT = /bot|crawl|spider|slurp|preview|facebookexternalhit|embedly|whatsapp|headless|lighthouse|pingdom|curl|wget|python|go-http|java\/|okhttp|axios|node-fetch/i;
const SEARCH = /(^|\.)(google|bing|yahoo|duckduckgo|baidu|yandex|ecosia|qwant|startpage|naver|seznam)\./;
const SOCIAL = /(^|\.)(facebook|fb|instagram|t|twitter|x|linkedin|lnkd|reddit|pinterest|tiktok|youtube|threads|bsky|mastodon)\.|^(t\.co|lnkd\.in)$/;
const EMAIL = /(^|\.)(mail|outlook|gmail)\./;

/** A real document navigation (counted once): not a prefetch, not a Next RSC / client-navigation fetch.
 * Those get the same decision (same header, no flicker), they're just not exposures. */
export function isDocument(req: Request, url = new URL(req.url)): boolean {
  const h = req.headers;
  if (/prefetch|prerender/.test(`${h.get("sec-purpose") ?? ""}${h.get("purpose") ?? ""}${h.get("x-purpose") ?? ""}`)) return false;
  if (h.has("next-router-prefetch") || h.has("rsc") || h.has("next-router-state-tree") || url.searchParams.has("_rsc")) return false;
  const dest = h.get("sec-fetch-dest");
  return dest ? dest === "document" : (h.get("accept") ?? "").includes("text/html");
}

export const isBot = (ua: string | null) => !ua || BOT.test(ua);
const cookie = (h: Headers, name: string) => new RegExp(`(?:^|;\\s*)${name}=([^;]*)`).exec(h.get("cookie") ?? "")?.[1];

/** Visitor context (contract §3): country, region, locale, device, source, visitor + app attributes. */
export function context(req: Request, o: Pick<DecideOptions, "locale" | "consent" | "attrs">, url = new URL(req.url)): Record<string, string> {
  const h = req.headers;
  const ctx: Record<string, string> = { ...o.attrs, locale: o.locale };
  const country = (h.get("x-vercel-ip-country") ?? h.get("cf-ipcountry") ?? h.get("cloudfront-viewer-country") ?? h.get("fastly-geo-country") ?? "").toUpperCase();
  if (/^[A-Z]{2}$/.test(country) && country !== "XX") ctx.country = country;
  const region = h.get("x-vercel-ip-country-region") ?? h.get("cf-region-code") ?? h.get("cloudfront-viewer-country-region");
  if (region) ctx.region = region.toUpperCase();
  const ua = h.get("user-agent") ?? "";
  ctx.device = /ipad|tablet|kindle|silk|android(?!.*mobile)/i.test(ua) ? "tablet"
    : h.get("sec-ch-ua-mobile") === "?1" || /mobi|iphone|ipod|android/i.test(ua) ? "mobile" : "desktop";
  ctx.source = source(url, h.get("referer"));
  ctx.visitor = o.consent ? (cookie(h, "glot_seen") ? "returning" : "new") : "unknown";
  return ctx;
}

function source(url: URL, referer: string | null): string {
  const q = url.searchParams;
  const medium = (q.get("utm_medium") ?? "").toLowerCase();
  if (/cpc|ppc|paid|display|cpm|banner|ads?$/.test(medium) || q.has("gclid") || q.has("fbclid") || q.has("msclkid")) return "paid";
  if (medium.includes("email") || medium === "newsletter") return "email";
  if (medium.includes("social")) return "social";
  if (medium === "organic") return "search";
  let host = (q.get("utm_source") ?? "").toLowerCase();
  if (host) host += "."; // `utm_source=google` → matches like a host
  else if (referer) {
    try {
      host = new URL(referer).hostname.replace(/^www\./, "");
    } catch {}
    if (host === url.hostname.replace(/^www\./, "")) return "direct"; // internal navigation
  }
  if (!host) return "direct";
  return SEARCH.test(host) ? "search" : SOCIAL.test(host) ? "social" : EMAIL.test(host) ? "email" : "referral";
}

/** Contract §3: conjunction; absent attribute ⇒ `eq`/`in` false, `not_in` true. */
export const matches = (rule: Rule, ctx: Record<string, string>) =>
  rule.all.every((p) => {
    const v = ctx[p.attr];
    if (p.not_in) return v === undefined || !p.not_in.includes(v);
    return v !== undefined && (p.eq !== undefined ? v === p.eq : !!p.in?.includes(v));
  });

/** FNV-1a → [0, 1). Deterministic across runtimes. */
export function hash(s: string): number {
  let x = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) x = Math.imul(x ^ s.charCodeAt(i), 0x01000193);
  return (x >>> 0) / 4294967296;
}

/** Index into `weights` for u ∈ [0, 1); -1 if every weight is 0. */
export function pick(weights: number[], u: number): number {
  let total = 0;
  for (const w of weights) total += w > 0 ? w : 0;
  if (!(total > 0)) return -1;
  let acc = 0, last = -1;
  for (let i = 0; i < weights.length; i++) {
    if (!(weights[i] > 0)) continue;
    last = i;
    if (u * total < (acc += weights[i])) return i;
  }
  return last; // float rounding at u ≈ 1
}

export function decide(req: Request, config: XpConfig | null, o: DecideOptions): Decision {
  const now = o.now ?? Date.now();
  if (!config || config.v !== 1 || !((config.fetched_at ?? Date.parse(config.generated_at)) + config.max_stale * 1000 > now)) return NONE;
  const h = req.headers;
  if (h.get("sec-gpc") === "1" || h.get("dnt") === "1") return NONE; // opt-out: official text, nothing counted
  const url = new URL(req.url);
  const salt = o.salt ?? "glot";
  const u = (k: string) => (o.unit !== undefined ? hash(`${salt}:${k}:${o.unit}`) : (o.rand ?? Math.random)());
  const session = config.persistence === "session";
  const saved = new Map<string, [number, string]>();
  if (session)
    for (const part of (cookie(h, "glot_xp") ?? "").split("~")) {
      const [id, arm, cell] = part.split(".");
      if (id && cell && /^\d$/.test(arm)) saved.set(id, [Number(arm), cell]);
    }

  const assignments: Assignment[] = [];
  const counted: Assignment[] = [];
  const overlay: Record<string, string> = {};
  const k: Record<string, string[]> = {};
  const apply = (xp: XpConfig["experiments"][number], i: number) => {
    const v = xp.arms[i].values?.[o.locale];
    if (v) Object.assign(overlay, v);
  };

  // Preview `?glot_xp=<id>.<arm>`: that arm only, nothing counted, nothing stored.
  const forced = url.searchParams.get("glot_xp");
  if (forced) {
    const [id, arm] = forced.split(".");
    const xp = config.experiments.find((x) => x.id === id);
    const i = xp ? xp.arms.findIndex((a) => a.arm === Number(arm)) : -1;
    if (!xp || i < 0) return NONE;
    apply(xp, i);
    return finish(o.locale, [{ id, cell: "*", arm: xp.arms[i].arm }], overlay, [], []);
  }

  let inHoldout = false;
  if (config.holdout) {
    const { id } = config.holdout;
    const prev = saved.get(id);
    const arm = prev && prev[1] === "*" ? prev[0] : u(id) * 100 < config.holdout.pct ? 0 : 1;
    inHoldout = arm === 0;
    assignments.push({ id, cell: "*", arm });
    if (!(prev && prev[0] === arm && prev[1] === "*")) counted.push({ id, cell: "*", arm });
  }

  if (!inHoldout) {
    const ctx = context(req, o, url);
    for (const xp of config.experiments) {
      if (!xp.locales.includes(o.locale)) continue;
      const prev = saved.get(xp.id);
      let cell = prev && xp.cells.find((c) => c.id === prev[1]);
      let i = cell ? xp.arms.findIndex((a) => a.arm === prev![0]) : -1;
      if (!cell || i < 0 || !(cell.weights[i] > 0)) {
        // ponytail: out-of-traffic visitors aren't stored, so in `session` mode they're re-drawn on each page.
        if (u(`${xp.id}:t`) * 100 >= xp.traffic_pct) continue;
        cell = xp.cells.find((c) => {
          const seg = c.id !== "*" && config.segments.find((s) => s.id === c.id);
          return c.id === "*" || (!!seg && matches(seg.rule, ctx));
        });
        if (!cell) continue;
        i = pick(cell.weights, u(xp.id));
        if (i < 0) continue;
      }
      const a = { id: xp.id, cell: cell.id, arm: xp.arms[i].arm };
      assignments.push(a);
      // `session`: once per experiment and session (an arm already in the cookie was counted on its first page).
      const live = cell.count && xp.status === "running";
      if (live && !(prev && prev[0] === a.arm && prev[1] === a.cell)) counted.push(a);
      if (live && xp.keys?.length) k[xp.id] = xp.keys; // only measurable experiments: no "seen" for previews/personalized cells
      apply(xp, i);
    }
  }

  const cookies: string[] = [];
  const secure = url.protocol === "https:" ? "; Secure" : "";
  if (session && assignments.length) {
    const kept = [...assignments];
    for (const [id, [arm, cell]] of saved) // keep arms of experiments not decided here (other locale), still live
      if (!kept.some((a) => a.id === id) && config.experiments.some((x) => x.id === id)) kept.push({ id, arm, cell });
    const value = kept.map((a) => `${a.id}.${a.arm}.${a.cell}`).join("~");
    if (value !== cookie(h, "glot_xp")) cookies.push(`glot_xp=${value}; Path=/; SameSite=Lax${secure}`);
  }
  if (o.consent && !cookie(h, "glot_seen")) cookies.push(`glot_seen=1; Path=/; Max-Age=31536000; SameSite=Lax${secure}`);
  const count = isDocument(req, url) && !isBot(h.get("user-agent"));
  return finish(o.locale, assignments, overlay, count ? counted : [], cookies, k);
}

function finish(locale: string, a: Assignment[], o: Record<string, string>, exposures: Assignment[], cookies: string[], k: Record<string, string[]> = {}): Decision {
  const x = Object.keys(k).length ? { v: 1, locale, a, o, k } : { v: 1, locale, a, o };
  return { assignments: a, overlay: o, exposures, cookies, header: a.length ? encode(JSON.stringify(x)) : null };
}

/** base64url, no padding, of UTF-8 (contract §7). */
export function encode(s: string): string {
  const b = new TextEncoder().encode(s);
  let bin = "";
  for (let i = 0; i < b.length; i += 0x8000) bin += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export type LoaderOptions = {
  /** glot server base URL. */
  server: string;
  /** Project slug. */
  project: string;
  /** Served from memory this long before a background revalidation (ETag). Default 60 s. */
  ttl?: number;
  /** Cold start: max wait for the first config, then official text. Default 150 ms. */
  timeout?: number;
  fetch?: typeof fetch;
};
type WaitUntil = (p: Promise<unknown>) => void;

/** `load(waitUntil?)` → the config, or null (none yet / glot unreachable / not public). Never throws. */
export function createExperimentsLoader({ server, project, ttl = 60_000, timeout = 150, fetch: f = globalThis.fetch }: LoaderOptions) {
  const url = `${server.replace(/\/+$/, "")}/cdn/${encodeURIComponent(project)}/experiments.json`;
  let hit: { at: number; etag?: string; config: XpConfig | null } | undefined;
  let inflight: Promise<XpConfig | null> | undefined;
  const refresh = () =>
    (inflight ??= (async () => {
      try {
        const res = await f(url, { headers: hit?.etag ? { "if-none-match": hit.etag } : {}, signal: AbortSignal.timeout(5000) });
        if (res.status === 304 && hit) {
          hit.at = Date.now();
          if (hit.config) hit.config.fetched_at = hit.at;
        } else if (res.ok) {
          const config = (await res.json()) as XpConfig;
          config.fetched_at = Date.now();
          hit = { at: config.fetched_at, etag: res.headers.get("etag") ?? undefined, config };
        }
        else hit = { at: Date.now(), config: null }; // 404/403: no experiments, ask again after ttl
      } catch {
        if (hit) hit.at = Date.now(); // unreachable: keep what we had (max_stale bounds it), retry after ttl
      } finally {
        inflight = undefined;
      }
      return hit?.config ?? null;
    })());

  return async function load(waitUntil?: WaitUntil): Promise<XpConfig | null> {
    if (hit) {
      if (Date.now() - hit.at >= ttl) {
        const p = refresh(); // stale-while-revalidate
        waitUntil?.(p);
      }
      return hit.config;
    }
    const p = refresh();
    waitUntil?.(p);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const late = new Promise<null>((r) => (timer = setTimeout(r, timeout, null)));
    return Promise.race([p, late]).finally(() => clearTimeout(timer));
  };
}

export type ExperimentsOptions = LoaderOptions & {
  /** Project public key (`pk_…`, dashboard → Insights): exposures go to `POST /api/insights/{key}`. */
  key: string;
  /** Your CMP's consent signal for this request. Default: none (no persistent cookie). */
  consent?: (req: Request) => boolean;
  /** How long exposures wait to be batched (ms, default 200; inside `waitUntil`, after the response). */
  batchMs?: number;
  /** App attributes for segments (`plan`, `logged_in`, `custom.*`). */
  attrs?: (req: Request) => Record<string, string> | undefined;
};

/** Load + decide + report exposures in the background. Shared by the Next proxy, the Worker and any edge runtime. */
export function glotExperiments(o: ExperimentsOptions) {
  const load = createExperimentsLoader(o);
  const endpoint = `${o.server.replace(/\/+$/, "")}/api/insights/${encodeURIComponent(o.key)}/batch`;
  const f = o.fetch ?? globalThis.fetch;
  // Exposures are queued per isolate and posted in batches (contract v1.3 `/batch`: no legacy `$view`).
  // ponytail: no retry (a lost batch = a few uncounted views), 50 items keeps a batch well under 64 KB.
  let queue: { served: string; exposures: Assignment[] }[] = [];
  let timer: Promise<void> | undefined;
  const flush = async () => {
    while (queue.length) {
      const body = JSON.stringify(queue.splice(0, 50));
      await f(endpoint, { method: "POST", body }).catch(() => {});
    }
  };
  return async (req: Request, locale: string, waitUntil?: WaitUntil): Promise<Decision> => {
    let d = NONE;
    try {
      d = decide(req, await load(waitUntil), { locale, consent: o.consent?.(req), attrs: o.attrs?.(req) });
    } catch {} // malformed config: official text
    if (d.exposures.length) {
      queue.push({ served: locale, exposures: d.exposures });
      const p = queue.length >= 50 ? flush() : (timer ??= new Promise<void>((r) => setTimeout(r, o.batchMs ?? 200)).then(() => ((timer = undefined), flush())));
      waitUntil?.(p);
    }
    return d;
  };
}

/** Request headers for the app: incoming `x-glot-xp` always dropped (client-forged), ours set when assigned. */
export function forwardHeaders(headers: Headers, d: Decision): Headers {
  const out = new Headers(headers);
  out.delete("x-glot-xp");
  if (d.header) out.set("x-glot-xp", d.header);
  return out;
}
