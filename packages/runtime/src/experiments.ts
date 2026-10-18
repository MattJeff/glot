// Segmented copy experiments, server side (contract §7): the edge (`@glot/edge`) draws the arms and forwards
// `x-glot-xp: base64url(JSON)`; `withExperiments` applies that overlay to the catalog inside `getRequestConfig`,
// so Server AND Client Components render the variant in the HTML — no flicker, no hydration mismatch.
// No runtime deps: runs in Node, the Edge runtime and Workers (plain `atob` + `TextDecoder`).
import { withVariants } from "./variants.ts";

/** One assignment: experiment (or holdout) id, segment cell (`*` = rest of traffic), arm (0 = official). */
export type Assignment = { id: string; cell: string; arm: number };
/** `k` (optional, contract v1.6): experiment id → its keys, to find the tested blocks in the page ("seen" exposure). */
export type XpHeader = { v: 1; locale: string; a: Assignment[]; o?: Record<string, string>; k?: Record<string, string[]> };

/** Published config (`GET /cdn/{slug}/experiments.json`), only what's needed to rebuild a missing overlay. */
export type XpConfig = {
  experiments: { id: string; locales: string[]; arms: { arm: number; values?: Record<string, Record<string, string>> }[] }[];
};

export interface WithExperimentsOptions {
  /** Raw `x-glot-xp` request header (`(await headers()).get("x-glot-xp")`). */
  header: string | null | undefined;
  /** Locale actually being served. When it differs from the edge's locale (e.g. on-demand catalog not ready yet),
   *  the overlay is skipped: official text, never a variant in the wrong language. */
  locale?: string;
  /** Experiments config: rebuilds the overlay when the edge sent assignments only (`o` absent). */
  config?: XpConfig | null;
}

/** Decodes `x-glot-xp`; null when absent or unreadable (→ official messages). */
export function readXpHeader(header: string | null | undefined): XpHeader | null {
  if (!header) return null;
  try {
    const bin = atob(header.replace(/-/g, "+").replace(/_/g, "/"));
    const x = JSON.parse(new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0))));
    if (x?.v !== 1 || typeof x.locale !== "string" || !Array.isArray(x.a)) return null;
    const a = (x.a as Assignment[]).filter((e) => typeof e?.id === "string" && typeof e.cell === "string" && Number.isInteger(e.arm));
    const o = x.o && typeof x.o === "object" ? Object.fromEntries(Object.entries(x.o).filter(([, v]) => typeof v === "string")) : undefined;
    const k = x.k && typeof x.k === "object" ? Object.fromEntries(Object.entries(x.k).filter(([, v]) => Array.isArray(v))) : undefined;
    return { v: 1, locale: x.locale, a, o: o as Record<string, string> | undefined, k: k as Record<string, string[]> | undefined };
  } catch {
    return null;
  }
}

/** Encodes an `x-glot-xp` value (tests, custom proxies). */
export function writeXpHeader(x: XpHeader): string {
  const bytes = new TextEncoder().encode(JSON.stringify(x));
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** `messages` with the edge's variant overlay applied (only existing string keys; arm 0 = untouched). */
export function withExperiments<M extends Record<string, unknown>>(messages: M, o: WithExperimentsOptions): M {
  const x = readXpHeader(o.header);
  if (!x || (o.locale && o.locale !== x.locale)) return messages;
  let overlay = x.o;
  if (!overlay && o.config) {
    overlay = {};
    for (const { id, arm } of x.a) {
      const e = o.config.experiments.find((e) => e.id === id);
      if (e?.locales.includes(x.locale)) Object.assign(overlay, e.arms.find((a) => a.arm === arm)?.values?.[x.locale]);
    }
  }
  return overlay ? withVariants(messages, overlay) : messages;
}

// ---------- "seen" exposure (browser) ----------

export type Seen = Assignment & { seen: true; visible_ms: number };
const BUCKETS = [1000, 2000, 5000, 10000, 30000, 60000];
/** Visible time rounded down to a coarse bucket (aggregated stats only, nothing personal). */
export const bucket = (ms: number) => BUCKETS.filter((b) => b <= ms).pop() ?? 0;

/**
 * Watches the blocks of each experiment (`[data-glot-key]`, or the Worker's `[data-glot]`, whose key is in `keys[id]`).
 * An experiment is "seen" once one of its blocks stayed ≥ 50 % visible for ≥ 1 s; its cumulative visible time is
 * reported (bucketed) when the page is hidden or left. `onSeen` fires at most once per experiment and document.
 */
// ponytail: reported on hide/pagehide only (time visible after a hide is lost); blocks added later are picked up
// by a MutationObserver on <body>.
export function observeSeen(a: Assignment[], keys: Record<string, string[]>, onSeen: (s: Seen) => void): () => void {
  const st = a
    .filter((x) => keys[x.id]?.length)
    .map((x) => ({ x, keys: new Set(keys[x.id]), on: new Set<Element>(), since: 0, total: 0, seen: false, sent: false, timer: 0 as unknown }));
  if (!st.length || typeof IntersectionObserver === "undefined") return () => {};
  const shown = () => document.visibilityState !== "hidden";
  const start = (s: (typeof st)[number]) => {
    if (s.since || !s.on.size || !shown()) return;
    s.since = Date.now();
    if (!s.seen) s.timer = setTimeout(() => (s.seen = s.since > 0), 1000);
  };
  const stop = (s: (typeof st)[number]) => {
    if (s.since) s.total += Date.now() - s.since;
    s.since = 0;
    clearTimeout(s.timer as number);
  };
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        const key = (e.target as HTMLElement).dataset.glotKey ?? (e.target as HTMLElement).dataset.glot;
        for (const s of st) {
          if (!key || !s.keys.has(key)) continue;
          if (e.isIntersecting && e.intersectionRatio >= 0.5) s.on.add(e.target);
          else s.on.delete(e.target);
          if (s.on.size) start(s);
          else stop(s);
        }
      }
    },
    { threshold: 0.5 },
  );
  const scan = () => document.querySelectorAll("[data-glot-key],[data-glot]").forEach((el) => io.observe(el));
  const mo = typeof MutationObserver === "undefined" ? null : new MutationObserver(scan);
  mo?.observe(document.body, { childList: true, subtree: true });
  scan();
  const flush = () => {
    for (const s of st) {
      stop(s);
      if (s.seen && !s.sent) {
        s.sent = true;
        onSeen({ ...s.x, seen: true, visible_ms: bucket(s.total) });
      }
    }
  };
  const vis = () => (shown() ? st.forEach(start) : flush());
  addEventListener("visibilitychange", vis);
  addEventListener("pagehide", flush);
  return () => {
    flush();
    io.disconnect();
    mo?.disconnect();
    removeEventListener("visibilitychange", vis);
    removeEventListener("pagehide", flush);
  };
}
