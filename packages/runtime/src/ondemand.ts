// Server-side on-demand locales: a locale the app doesn't ship is fetched from the glot CDN with `?ondemand=1`.
// The first request queues the translation on the glot server (202) → `null`: render the source, the browser's
// on-device fallback covers the visitor. Once glot has the catalog, the server renders it (real HTML, SEO).
// Plain `fetch`, no dependencies: works in Node, Next (Node or Edge runtime) and Cloudflare Workers.
type Json = { [k: string]: unknown };

export interface OnDemandOptions {
  /** glot server base URL, e.g. `https://glot.example.com`. */
  server: string;
  /** Project slug (its CDN must be public, and on-demand enabled for the locale). */
  project: string;
  /** Source catalog: missing keys in a partial catalog fall back to it (deep merge). */
  source?: Json;
  /** How long a catalog is served from memory before revalidating (ETag) in the background. Default 60 s. */
  ttl?: number;
  fetch?: typeof fetch;
}

type Entry = { at: number; ttl: number; etag?: string; messages: Json | null };

/** Returns `load(locale)`: the translated catalog, or `null` while glot translates it (or it's not allowed). */
export function createOnDemandLoader({ server, project, source, ttl = 60_000, fetch: f = globalThis.fetch }: OnDemandOptions) {
  const base = server.replace(/\/+$/, "");
  const cache = new Map<string, Entry>();
  const inflight = new Map<string, Promise<Json | null>>();

  const refresh = (locale: string) => {
    const running = inflight.get(locale);
    if (running) return running;
    const prev = cache.get(locale);
    const p = (async () => {
      try {
        const res = await f(`${base}/cdn/${encodeURIComponent(project)}/${encodeURIComponent(locale)}.json?ondemand=1`, {
          headers: prev?.etag ? { "if-none-match": prev.etag } : {},
        });
        if (res.status === 304 && prev) {
          prev.at = Date.now();
          return prev.messages;
        }
        const messages = res.status === 200 ? merge(source ?? {}, (await res.json()) as Json) : null;
        // 202 / 4xx: ask again soon (Retry-After, capped by ttl) — the next request after that may be translated.
        const retry = (Number(res.headers.get("retry-after") ?? 5) || 0) * 1000;
        cache.set(locale, { at: Date.now(), ttl: messages ? ttl : Math.min(ttl, retry), etag: res.headers.get("etag") ?? undefined, messages });
        return messages;
      } catch {
        return prev?.messages ?? null; // glot unreachable: keep serving what we had
      } finally {
        inflight.delete(locale);
      }
    })();
    inflight.set(locale, p);
    return p;
  };

  return async function load(locale: string): Promise<Json | null> {
    const hit = cache.get(locale);
    if (!hit) return refresh(locale);
    if (Date.now() - hit.at < hit.ttl) return hit.messages;
    if (!hit.messages) return refresh(locale); // still translating: a cheap 202 is worth waiting for
    void refresh(locale); // stale-while-revalidate
    return hit.messages;
  };
}

/** `Accept-Language` → language tags by preference (`*` and q=0 dropped). */
export function acceptedLanguages(header: string | null | undefined): string[] {
  return (header ?? "")
    .split(",")
    .map((part) => {
      const [tag, ...params] = part.trim().split(";");
      const q = params.map((p) => p.trim()).find((p) => p.startsWith("q="));
      return { tag: tag.trim(), q: q ? Number(q.slice(2)) : 1 };
    })
    .filter((l) => l.tag && l.tag !== "*" && l.q > 0)
    .sort((a, b) => b.q - a.q)
    .map((l) => l.tag);
}

function merge(a: Json, b: Json): Json {
  const out: Json = { ...a };
  for (const [k, v] of Object.entries(b)) {
    const prev = out[k];
    out[k] = isObj(prev) && isObj(v) ? merge(prev, v) : v;
  }
  return out;
}
const isObj = (v: unknown): v is Json => typeof v === "object" && v !== null && !Array.isArray(v);
