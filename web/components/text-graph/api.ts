// Text graph API (crates/server/src/text_graph.rs). Session auth, prefix /api/projects/{slug}/text-graph.
import { qs, request, type TranslationStatus } from "@/lib/api";

/** A sentence: one identity wherever it appears. */
export type AnchorSummary = { id: string; text: string; context: string; occurrences: number; surfaces: string[] };
/** One translation of a sentence and the places (`surface:locator`) showing it. */
export type ValueGroup = { value: string; count: number; reviewed: boolean; places: string[] };
export type LocaleStateName = "missing" | "consistent" | "partial" | "inconsistent" | "conflict";
export type SkipReason = "reviewed_conflict" | "machine_disagrees" | "needs_review" | "placeholders" | "validation";
export type GraphAction = {
  anchor_id: string;
  surface: string;
  locator: string;
  locale: string;
  value: string;
  previous: string | null;
  action: "write" | "skip";
  reason: SkipReason | null;
  detail: string | null;
};
export type Inconsistency = { anchor_id: string; text: string; locale: string; state: "inconsistent" | "conflict"; values: ValueGroup[] };
export type Overview = {
  synced_at: string | null;
  anchors: number;
  occurrences: number;
  surfaces: Record<string, number>;
  shared: number;
  cross_surface: number;
  inconsistent: number;
  conflicts: number;
  inconsistencies: Inconsistency[];
  fillable: number;
  blocked: number;
  reworded: { from: { id: string; text: string }; to: { id: string; text: string }; moved: number; remaining: number }[];
};
export type Occurrence = {
  surface: string;
  locator: string;
  text: string;
  role: string;
  meta: { url?: string | null; hits?: number; account?: string; kind?: string; field?: string };
  translations: Record<string, { value: string; status: Exclude<TranslationStatus, "missing"> | "pending"; origin: string; inherited: boolean }>;
};
export type LocaleState = {
  locale: string;
  state: LocaleStateName;
  values: ValueGroup[];
  missing: number;
  winner: string | null;
  fillable: number;
  blocked: GraphAction[];
};
export type AnchorDetail = {
  id: string;
  text: string;
  context: string;
  occurrences: Occurrence[];
  locales: LocaleState[];
  related: { id: string; text: string; context: string; kind: "context" | "near" | "fuzzy"; score: number; occurrences: number }[];
  moved: { surface: string; locator: string; to: { id: string; text: string } }[];
};
export type WriteResult = { dry_run: boolean; written: number; writes: number; skipped: number; actions: GraphAction[] };

const base = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/text-graph`;

export const textGraphApi = {
  overview: (slug: string) => request<Overview>("GET", base(slug)),
  search: (slug: string, q: string) => request<{ anchors: AnchorSummary[] }>("GET", `${base(slug)}/search${qs({ q, limit: 50 })}`),
  anchor: (slug: string, id: string) => request<AnchorDetail>("GET", `${base(slug)}/anchors/${id}`),
  unify: (slug: string, id: string, b: { locale: string; value: string; force?: boolean; dry_run?: boolean }) =>
    request<WriteResult>("POST", `${base(slug)}/anchors/${id}/unify`, b),
  propagate: (slug: string) => request<WriteResult>("POST", `${base(slug)}/propagate`, {}),
  sync: (slug: string) => request<{ occurrences: number }>("POST", `${base(slug)}/sync`, {}),
};

export const textGraphKeys = {
  all: (slug: string) => ["text-graph", slug] as const,
  overview: (slug: string) => ["text-graph", slug, "overview"] as const,
  search: (slug: string, q: string) => ["text-graph", slug, "search", q] as const,
  anchor: (slug: string, id: string) => ["text-graph", slug, "anchor", id] as const,
};
