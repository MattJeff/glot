// glot Edge dashboard API (crates/server/src/edge/*). Session auth, prefix /api/projects/{slug}/edge.
// One resource per server module: settings (discover.rs), domains (proxy/ask), segments (tm.rs),
// coverage (critical pages + locales.json gate), seo (seo_report.rs), metrics (proxy/worker counters).
import { ApiError, type Issue, qs, request } from "@/lib/api";

export type EdgeMode = "subdomain" | "subdirectory" | "middleware";
export const EDGE_MODES: EdgeMode[] = ["subdomain", "subdirectory", "middleware"];
export type Rendering = "origin" | "prerender" | "auto";

/** edge::discover::Settings — PUT replaces the whole document. */
export type EdgeSettings = {
  /** Project kill switch: false = every domain passes the origin through untouched. */
  enabled: boolean;
  auto_translate: boolean;
  /** Path globs (`/blog/*`) where discovery is allowed; empty = every public path. */
  discover_include: string[];
  discover_exclude: string[];
  selectors_include: string[];
  selectors_exclude: string[];
  /** Critical pages: gate of `locales.json` `ready`. */
  critical_patterns: string[];
  /** Pages whose new segments wait for a review before being served. */
  sensitive_patterns: string[];
  /** 0..1 */
  critical_threshold: number;
  daily_cap: number;
  monthly_budget_usd: number | null;
  rendering: Rendering;
};

export type DnsCheck = { ok: boolean; expected: string; found: string[] };
export type EdgeDomain = {
  id: string;
  host: string;
  mode: EdgeMode;
  locale: string;
  origin_url: string;
  /** Per-domain kill switch. */
  enabled: boolean;
  verified_at: string | null;
  /** TXT value to publish at `_glot.{host}`. */
  verify_token: string;
  /** CNAME target for subdomain mode (edge.glotlang.com). */
  cname_target?: string;
  tls?: "pending" | "active" | "error" | null;
  checks?: { cname?: DnsCheck; txt?: DnsCheck } | null;
  checked_at?: string | null;
  created_at: string;
};
export type NewEdgeDomain = { host: string; mode: EdgeMode; locale: string; origin_url: string };

/** GET /edge/coverage (seo_report.rs): readiness per locale with every page's coverage, critical pages first. */
export type MissingSegment = { hash: string; context: string; text: string | null };
export type EdgePage = { path: string; critical: boolean; segments: number; translated: number; coverage: number; missing: MissingSegment[] };
export type EdgeLocale = {
  locale: string;
  /** 0..1, keys + segments. */
  coverage: number;
  /** 0..1, lowest critical page; null = no critical page seen yet. */
  critical_coverage: number | null;
  critical_threshold: number;
  /** Shown in the switcher / locales.json manifest. */
  ready: boolean;
  version: number;
  pages: EdgePage[];
};
export type EdgeCoverage = { seo_min: number; locales: EdgeLocale[] };

/** GET /edge/segments (tm.rs): one locale, `status` = missing when there is no translation yet. */
export type SegmentStatus = "missing" | "pending" | "machine" | "reviewed" | "stale";
export const SEGMENT_STATUSES: SegmentStatus[] = ["missing", "pending", "machine", "reviewed", "stale"];
export type EdgeSegment = {
  hash: string;
  /** `<role>|<page template>` (glot_core::edge::tm::context). */
  context: string;
  source: string;
  url_sample: string | null;
  hits: number;
  needs_review: boolean;
  first_seen: string;
  last_seen: string;
  value: string | null;
  status: SegmentStatus;
  issues: Issue[];
};
export type SegmentQuery = { locale: string; status?: SegmentStatus; path?: string; limit?: number };

/** GET/POST /edge/seo-report: raw fetch (no JavaScript) of every advertised locale of up to 20 pages. */
export type SeoLocaleResult = {
  locale: string;
  url: string;
  status: number | null;
  error: string | null;
  final_url: string | null;
  lang: string | null;
  dir: string | null;
  content_language: string | null;
  canonical: string | null;
  /** locale (lowercase, x-default) → absolute URL, as the page declares it. */
  hreflang: Record<string, string>;
  not_reciprocal: string[];
  advertised: boolean;
  title: string | null;
  description: string | null;
  og_title: string | null;
  jsonld: number;
  untranslated: { count: number; segments: number; sample: { hash: string; text: string; role: string }[] };
  noindex: boolean;
  robots_blocked: boolean;
  technically_indexable: boolean;
  /** Actually indexed: known only with Search Console; null = unknown, never guessed. */
  indexed: boolean | null;
  issues: string[];
};
export type SeoReportPage = {
  path: string;
  critical: boolean;
  /** Raw HTML vs rendered DOM of the source page (renderer). */
  render: { status: "same" | "js_only_content" | "unknown"; reason?: string; only_after_js?: number; sample?: string[] };
  locales: SeoLocaleResult[];
};
export type SeoReport = {
  generated_at: string;
  indexed: { status: string; reason?: string };
  sitemaps: string[];
  issues: Record<string, number>;
  pages: SeoReportPage[];
};

export type EdgeMetrics = {
  days: number;
  requests: number;
  /** 0..1 */
  cache_hit_ratio: number;
  fail_open: number;
  /** glot's own cost on top of the origin (Server-Timing), ms. */
  glot_p50_ms: number | null;
  glot_p95_ms: number | null;
  origin_p50_ms: number | null;
  by_day: { date: string; requests: number; hits: number; fail_open: number; glot_p95_ms: number | null }[];
};

const enc = encodeURIComponent;
const e = (slug: string) => `/api/projects/${enc(slug)}/edge`;
export const edgeApi = {
  settings: (slug: string) => request<EdgeSettings>("GET", `${e(slug)}/settings`),
  putSettings: (slug: string, b: EdgeSettings) => request<EdgeSettings>("PUT", `${e(slug)}/settings`, b),
  // ponytail: domains + metrics are the dashboard contract; no server route yet (see report).
  domains: (slug: string) => request<EdgeDomain[]>("GET", `${e(slug)}/domains`),
  addDomain: (slug: string, b: NewEdgeDomain) => request<EdgeDomain>("POST", `${e(slug)}/domains`, b),
  updateDomain: (slug: string, id: string, b: { enabled: boolean }) => request<EdgeDomain>("PATCH", `${e(slug)}/domains/${enc(id)}`, b),
  deleteDomain: (slug: string, id: string) => request<void>("DELETE", `${e(slug)}/domains/${enc(id)}`),
  verifyDomain: (slug: string, id: string) => request<EdgeDomain>("POST", `${e(slug)}/domains/${enc(id)}/verify`),
  metrics: (slug: string, days = 7) => request<EdgeMetrics>("GET", `${e(slug)}/metrics${qs({ days })}`),
  coverage: (slug: string) => request<EdgeCoverage>("GET", `${e(slug)}/coverage`),
  segments: (slug: string, q: SegmentQuery) => request<{ version: number; segments: EdgeSegment[] }>("GET", `${e(slug)}/segments${qs(q)}`),
  /** Human translation = approval (`reviewed`); 422 validation_error when it breaks syntax/glossary. */
  putSegment: (slug: string, hash: string, locale: string, b: { context: string; value: string }) =>
    request<{ status: SegmentStatus; issues: Issue[]; version: number }>("PUT", `${e(slug)}/segments/${enc(hash)}/${enc(locale)}`, b),
  /** null = no report yet (404). */
  seoReport: (slug: string) =>
    request<SeoReport>("GET", `${e(slug)}/seo-report`).catch((err) => {
      if (err instanceof ApiError && err.status === 404) return null;
      throw err;
    }),
  runSeoReport: (slug: string) => request<SeoReport>("POST", `${e(slug)}/seo-report`),
  /** Submits the ready locales' sitemap URLs to IndexNow (Bing, Yandex; Google reads the sitemaps). */
  indexNow: (slug: string) =>
    request<{ hosts: { host: string; urls: number; status: number | null }[] }>("POST", `${e(slug)}/indexnow`),
  /** `<link rel="alternate">` tags the source site must carry itself (subdomain mode). */
  hreflang: (slug: string, path: string) => request<{ path: string; tags: string[] }>("GET", `${e(slug)}/hreflang${qs({ path })}`),
};

export const edgeKeys = {
  all: (slug: string) => ["edge", slug] as const,
  settings: (slug: string) => ["edge", slug, "settings"] as const,
  domains: (slug: string) => ["edge", slug, "domains"] as const,
  coverage: (slug: string) => ["edge", slug, "coverage"] as const,
  segments: (slug: string) => ["edge", slug, "segments"] as const,
  seo: (slug: string) => ["edge", slug, "seo"] as const,
  metrics: (slug: string) => ["edge", slug, "metrics"] as const,
};

/** Split a segment context `<role>|<template>` for display. */
export const splitContext = (c: string) => {
  const i = c.indexOf("|");
  return i < 0 ? { role: c, template: "" } : { role: c.slice(0, i), template: c.slice(i + 1) };
};
