// glot <-> Framer localization sync, shared by the plugin (src/main.ts, inside the Framer editor) and the
// Server API script (src/cli.ts, from any server). Both expose the same localization methods, typed here
// structurally so the core runs against either SDK, or a test double.
//
// One run: read Framer's localization groups -> push their source strings to glot (one key per source,
// `framer:{group type}:{group id}:{source id}`) -> optionally run a glot translate job -> write back the
// translations glot considers valid (machine or reviewed, never stale) with `setLocalizationData`.

export type FramerLocale = { id: string; code: string; name?: string };
export type FramerValue = { status: "new" | "needsReview" | "done" | "warning"; value: string | null; readonly?: boolean };
export type FramerSource = {
  id: string;
  type: "string" | "formattedText" | "altText" | "slug" | "link";
  value: string;
  valueByLocale: Record<string, FramerValue>;
};
export type FramerGroup = {
  id: string;
  name: string;
  type: string;
  sources: FramerSource[];
  statusByLocale?: Record<string, "excluded" | "ready">;
};
export type ValueUpdate = { action: "set"; value: string; needsReview?: boolean };

/** The subset of the Plugin / Server API this sync uses. */
export interface FramerLike {
  getLocales(): Promise<readonly FramerLocale[]>;
  getDefaultLocale(): Promise<FramerLocale>;
  getLocalizationGroups(): Promise<readonly FramerGroup[]>;
  setLocalizationData(data: { valuesBySource: Record<string, Record<string, ValueUpdate>> }): Promise<{
    valuesBySource: { errors: readonly { sourceId: string; localeId: string | null; error: string }[] };
  }>;
}

export type GlotOptions = {
  /** e.g. https://app.glotlang.com (no trailing slash). */
  base: string;
  /** glot API token (`glot_…`, dashboard -> Settings -> API tokens). */
  token: string;
  /** Project slug. */
  project: string;
  /** `/api/plugin` from the browser (CORS-enabled, token-only routes), `/api` from a server. */
  prefix?: string;
  fetch?: typeof fetch;
};

export type SyncOptions = {
  /** Start a glot translate job for the pushed keys and wait for it (default true). */
  translate?: boolean;
  /** Overwrite values a person marked done in Framer (default false). */
  force?: boolean;
  /** Max wait for the translate job (default 10 min). */
  timeoutMs?: number;
  onProgress?: (message: string) => void;
};

export type SyncReport = {
  sources: number;
  created: number;
  updated: number;
  written: number;
  /** Translations not written: stale, missing, or a person's value kept in Framer. */
  skipped: number;
  errors: string[];
  /** Framer locales with no matching glot target locale. */
  unmatchedLocales: string[];
};

/** Source types sent to glot. Slugs and links are left to Framer (URLs, not prose). */
const TEXT_TYPES = new Set(["string", "formattedText", "altText"]);

export const keyOf = (g: FramerGroup, s: FramerSource) => `framer:${g.type}:${g.id}:${s.id}`;

/** glot_core::hash::hash: first 16 hex chars of sha256(text). */
export async function hash(text: string): Promise<string> {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
  return Array.from(d.slice(0, 8), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Framer locale code -> glot locale: exact (case-insensitive), else same language (`fr-FR` -> `fr`, `fr` -> `fr-FR`). */
export function matchLocale(code: string, glot: string[]): string | undefined {
  const n = (s: string) => s.toLowerCase().replace("_", "-");
  const exact = glot.find((l) => n(l) === n(code));
  if (exact) return exact;
  const lang = n(code).split("-")[0];
  return glot.find((l) => n(l).split("-")[0] === lang);
}

type Lock = Record<string, Record<string, { source_hash: string; origin: { kind: string }; reviewed?: boolean }>>;
type Pull = { source_locale: string; catalogs: Record<string, Record<string, string>>; lock: { entries?: Lock } };

export function glotClient(o: GlotOptions) {
  const f = o.fetch ?? fetch;
  const api = `${o.base.replace(/\/$/, "")}${o.prefix ?? "/api"}`;
  const call = async <T>(method: string, path: string, body?: unknown): Promise<T> => {
    const res = await f(`${api}${path}`, {
      method,
      headers: { authorization: `Bearer ${o.token}`, ...(body ? { "content-type": "application/json" } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    if (!res.ok) {
      let msg = text.slice(0, 300);
      try {
        msg = JSON.parse(text).error?.message ?? JSON.parse(text).message ?? msg;
      } catch {}
      throw new Error(`glot ${method} ${path}: HTTP ${res.status} ${msg}`);
    }
    return (text ? JSON.parse(text) : {}) as T;
  };
  const p = `/projects/${encodeURIComponent(o.project)}`;
  return {
    pull: () => call<Pull>("GET", `${p}/sync/pull`),
    push: (sourceLocale: string, catalog: Record<string, string>, context: Record<string, { description: string }>) =>
      call<{ created: number; updated: number }>("POST", `${p}/sync/push`, { catalogs: { [sourceLocale]: catalog }, context }),
    translate: (keys: string[], locales: string[]) =>
      call<{ id: string; status: string }>("POST", `${p}/jobs`, { kind: "translate", keys, locales }),
    job: (id: string) => call<{ id: string; status: string; error?: string | null }>("GET", `/jobs/${id}`),
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function sync(framer: FramerLike, glot: ReturnType<typeof glotClient>, opts: SyncOptions = {}): Promise<SyncReport> {
  const log = opts.onProgress ?? (() => {});
  const report: SyncReport = { sources: 0, created: 0, updated: 0, written: 0, skipped: 0, errors: [], unmatchedLocales: [] };

  const [locales, def, groups, first] = await Promise.all([
    framer.getLocales(),
    framer.getDefaultLocale(),
    framer.getLocalizationGroups(),
    glot.pull(),
  ]);
  const targets = Object.keys(first.catalogs).filter((l) => l !== first.source_locale);

  // 1. Sources -> glot.
  const catalog: Record<string, string> = {};
  const context: Record<string, { description: string }> = {};
  const sources: { key: string; group: FramerGroup; source: FramerSource }[] = [];
  for (const group of groups)
    for (const source of group.sources) {
      if (!TEXT_TYPES.has(source.type) || !source.value?.trim()) continue;
      const key = keyOf(group, source);
      catalog[key] = source.value;
      const html = source.type === "formattedText" ? " HTML: translate the text only, keep every tag and attribute." : "";
      const alt = source.type === "altText" ? " (image alt text)" : "";
      context[key] = { description: `Framer ${group.type} “${group.name}”${alt}.${html}` };
      sources.push({ key, group, source });
    }
  report.sources = sources.length;
  if (!sources.length) return report;
  log(`Sending ${sources.length} strings to glot…`);
  const pushed = await glot.push(first.source_locale, catalog, context);
  report.created = pushed.created;
  report.updated = pushed.updated;

  // Framer locale -> glot locale.
  const pairs: { framer: FramerLocale; glot: string }[] = [];
  for (const l of locales) {
    if (l.id === def.id) continue;
    const g = matchLocale(l.code, targets);
    if (g) pairs.push({ framer: l, glot: g });
    else report.unmatchedLocales.push(l.code);
  }
  if (!pairs.length) return report;

  // 2. Translate (only what is missing or stale: the job skips up-to-date translations).
  if (opts.translate ?? true) {
    log("Translating…");
    const job = await glot.translate(Object.keys(catalog), pairs.map((p) => p.glot));
    const deadline = Date.now() + (opts.timeoutMs ?? 600_000);
    let status = job.status;
    for (let wait = 500; status !== "done" && status !== "failed"; wait = Math.min(wait * 2, 5000)) {
      if (Date.now() > deadline) {
        report.errors.push("translation still running: run the sync again in a minute to write the rest");
        break;
      }
      await sleep(wait);
      const j = await glot.job(job.id);
      status = j.status;
      if (status === "failed") report.errors.push(`translation job failed: ${j.error ?? "unknown error"}`);
    }
  }

  // 3. Valid translations -> Framer.
  const { catalogs, lock } = await glot.pull();
  const entries = lock.entries ?? {};
  const valuesBySource: Record<string, Record<string, ValueUpdate>> = {};
  for (const { key, source } of sources) {
    const srcHash = await hash(source.value);
    for (const { framer: fl, glot: gl } of pairs) {
      const value = catalogs[gl]?.[key];
      const entry = entries[gl]?.[key];
      const current = source.valueByLocale[fl.id];
      // Stale (source changed since) or missing: never written.
      if (!value || !entry || entry.source_hash !== srcHash) {
        report.skipped++;
        continue;
      }
      // Already there; only a review done in glot since is carried over (needsReview -> done).
      if (current?.value === value && !(entry.reviewed && current.status === "needsReview")) continue;
      // A value a person finished in Framer (or one Framer locks, e.g. managed collections) is kept.
      if (current?.readonly || (current?.status === "done" && current.value && !opts.force)) {
        report.skipped++;
        continue;
      }
      // Reviewed in glot = done in Framer; machine translations stay flagged for review.
      (valuesBySource[source.id] ??= {})[fl.id] = { action: "set", value, needsReview: !entry.reviewed };
      report.written++;
    }
  }
  if (report.written) {
    log(`Writing ${report.written} translations to Framer…`);
    const res = await framer.setLocalizationData({ valuesBySource });
    for (const e of res.valuesBySource.errors) {
      report.written--;
      report.errors.push(`${e.sourceId}${e.localeId ? ` (${e.localeId})` : ""}: ${e.error}`);
    }
  }
  return report;
}
