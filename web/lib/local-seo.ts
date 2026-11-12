// Client for the Local SEO API (crates/server/src/local_seo). Kept apart from lib/api.ts: one feature, one file.
import { request } from "@/lib/api";

export type SeoFaq = { question: string; answer: string };
/** Search-facing content of one page in one market; an absent field keeps the faithful translation. */
export type SeoContent = {
  title: string | null;
  description: string | null;
  h1: string | null;
  h2: string[];
  slug: string | null;
  faq: SeoFaq[];
};
export type SeoIssue = { code: string; severity: "error" | "warning"; field: string; message: string };
/** `impressions` is null when unknown: never estimated. */
export type SeoTerm = { term: string; source: "manual" | "gsc" | "gsc_site"; impressions: number | null; clicks: number | null; position: number | null };
export type SeoWindow = { days: number; impressions: number; clicks: number; position: number | null };
export type SeoStatus = "proposed" | "approved" | "auto" | "rejected" | "stale" | "needs_demand";
export type SeoRow = {
  id: string;
  path: string;
  locale: string;
  status: SeoStatus;
  url: string | null;
  source: { title: string; description: string; h1: string };
  current: SeoContent;
  proposed: SeoContent;
  edited: SeoContent | null;
  effective: SeoContent;
  terms: SeoTerm[];
  rationale: string;
  /** Validators on the proposal / the edit. */
  issues: SeoIssue[];
  /** The same validators on what the market shows today (needs no search data). */
  audit: SeoIssue[];
  model: string | null;
  decided_at: string | null;
  applied_at: string | null;
  updated_at: string;
  /** null until the override went live; windows are null until Search Console rows exist. */
  metrics: { before: SeoWindow | null; after: SeoWindow | null } | null;
};
export type SeoMarket = {
  locale: string;
  language: string;
  country: string | null;
  engine: "google" | "baidu" | "yandex" | "naver";
  limits: { title: number; description: number };
};
export type LocalSeo = {
  mode: "review" | "auto";
  site: { origin: string | null; saved_url: string | null; edge: boolean };
  llm_provider: string;
  gsc: { configured: boolean; connected: boolean; site_url: string | null; connected_at: string | null };
  markets: SeoMarket[];
  pages: { path: string; title: string; scanned_at: string }[];
  manual_demand: { locale: string; path: string; term: string; impressions: number | null }[];
  rows: SeoRow[];
};
export type SeoScan = {
  pages: number;
  results: { path: string; locale: string; status: SeoStatus; note: string | null }[];
  warnings: string[];
};

const base = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/local-seo`;

export const localSeo = {
  key: (slug: string) => ["local-seo", slug] as const,
  get: (slug: string) => request<LocalSeo>("GET", base(slug)),
  scan: (slug: string, b: { paths?: string[]; regenerate?: boolean }) => request<SeoScan>("POST", `${base(slug)}/scan`, b),
  settings: (slug: string, b: { mode?: "review" | "auto"; site_url?: string }) => request<unknown>("PUT", `${base(slug)}/settings`, b),
  demand: (slug: string, b: { locale: string; path?: string; terms: { term: string; impressions?: number }[] }) =>
    request<{ saved: number }>("PUT", `${base(slug)}/demand`, b),
  approve: (slug: string, id: string) => request<unknown>("POST", `${base(slug)}/overrides/${id}/approve`),
  reject: (slug: string, id: string) => request<unknown>("POST", `${base(slug)}/overrides/${id}/reject`),
  edit: (slug: string, id: string, content: SeoContent, approve: boolean) =>
    request<{ status: SeoStatus; issues: SeoIssue[] }>("PATCH", `${base(slug)}/overrides/${id}`, { content, approve }),
  gscConnect: (slug: string) => request<{ url: string }>("GET", `${base(slug)}/gsc/connect`),
  gscDisconnect: (slug: string) => request<void>("DELETE", `${base(slug)}/gsc`),
  refreshMetrics: (slug: string) => request<{ days: number; errors: string[] }>("POST", `${base(slug)}/metrics/refresh`),
};

/** "term, 1200" per line → terms. A trailing number is the monthly volume from the customer's own tool. */
export function parseTerms(text: string) {
  return text
    .split("\n")
    .map((line) => {
      const m = line.match(/^(.*?)[,;\t]\s*(\d[\d\s.,]*)$/);
      const term = (m ? m[1] : line).trim();
      const n = m ? Number(m[2].replace(/[\s.,]/g, "")) : NaN;
      return Number.isFinite(n) ? { term, impressions: n } : { term };
    })
    .filter((t) => t.term);
}
