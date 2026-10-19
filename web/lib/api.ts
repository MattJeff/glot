// Typed client for the glot-server HTTP API (see ARCHITECTURE.md § API HTTP).
// Requests go through the Next.js rewrite (/api → :8790), so the session cookie is same-origin.

export type User = { id: string; email: string; name: string };
/** GET /api/me. email_enabled = the server can send email (RESEND_API_KEY set). */
export type Me = User & { email_verified: boolean; email_enabled: boolean; locale: string; notify_job_done: boolean };
export type ApiToken = { id: string; name: string; created_at: string; last_used_at: string | null };

export type Provider = "anthropic" | "openai" | "openai-compatible" | "mock";
export type LocaleStats = { translated: number; reviewed: number; stale: number; issues: number };
export type Project = {
  id: string;
  slug: string;
  name: string;
  source_locale: string;
  target_locales: string[];
  style: string | null;
  public_cdn: boolean;
  created_at: string;
  llm: {
    provider: Provider;
    model: string;
    fallback_model: string | null;
    judge_model: string | null;
    has_api_key: boolean;
    budget_usd: number | null;
    anthropic_workspace_id?: string | null;
    extra_headers?: Record<string, string>;
  };
  stats: { keys: number; locales: Record<string, LocaleStats> };
};
export type LlmSettings = {
  provider: Provider;
  model: string;
  fallback_model?: string | null;
  judge_model?: string | null;
  api_key?: string;
  base_url?: string | null;
  budget_usd?: number | null;
  /** "" clears; absent keeps. */
  anthropic_workspace_id?: string;
  /** Non-secret headers; absent keeps. */
  extra_headers?: Record<string, string>;
};
export type ProjectPatch = {
  name?: string;
  source_locale?: string;
  target_locales?: string[];
  llm?: LlmSettings;
  style?: string;
  public_cdn?: boolean;
};

export type Severity = "error" | "warning";
export type Issue = { code: string; severity: Severity; message: string };
export type TranslationStatus = "missing" | "machine" | "reviewed" | "stale";
export const STATUSES: TranslationStatus[] = ["missing", "machine", "reviewed", "stale"];
export type Translation = {
  value: string;
  status: TranslationStatus;
  score: number | null;
  issues: Issue[];
  origin: "machine" | "human" | "imported";
  model: string | null;
  updated_at: string;
};
export type KeyRow = {
  key: string;
  description: string | null;
  source: string;
  refs: { path: string; line: number }[];
  translations: Record<string, Translation>;
};
export type KeyPage = { items: KeyRow[]; total: number };
export type KeyQuery = { q?: string; locale?: string; status?: TranslationStatus; page?: number; per_page?: number };
export type HistoryEntry = {
  locale: string;
  value: string;
  origin: string;
  model: string | null;
  score: number | null;
  created_at: string;
  author: string | null;
};

export type Term = {
  id: string;
  term: string;
  description: string | null;
  do_not_translate: boolean;
  case_sensitive: boolean;
  translations: Record<string, string>;
  forbidden: Record<string, string[]>;
};
export type TermInput = Omit<Term, "id">;

export type ProjectIssue = { key: string; locale: string; issue: Issue };

export type JobStatus = "queued" | "running" | "done" | "failed";
export type Job = {
  id: string;
  kind: "translate";
  status: JobStatus;
  progress: { done: number; total: number };
  cost_usd: number;
  error: string | null;
  created_at: string;
  finished_at: string | null;
};

export type Verdict = "same" | "nuance" | "different";
export type Review = {
  key: string;
  locale: string;
  source: string;
  target: string;
  back_translation: string;
  verdict: Verdict;
  explanation: string;
  suggestion: string | null;
  cached: boolean;
  created_at: string;
};

export type VisualKind = "overflow" | "clamp" | "overlap" | "untranslated" | "tofu" | "rtl";
/** Most severe first: broken layout, unreadable glyphs, wrong direction, untranslated, then truncation the CSS asks for. */
export const VISUAL_KINDS: VisualKind[] = ["overflow", "overlap", "tofu", "rtl", "untranslated", "clamp"];
export type Rect = { x: number; y: number; width: number; height: number };
export type VisualIssue = {
  kind: VisualKind;
  locale: string;
  route: string;
  viewport: string;
  selector: string;
  text: string;
  key?: string;
  rect?: Rect;
  crop?: string;
  suggestion?: string;
  command?: string;
};
export type VisualPage = { locale: string; route: string; viewport: string; url: string; screenshot: string };
export type VisualSummary = {
  pages: number;
  issues: number;
  locales: string[];
  routes: string[];
  viewports: string[];
  by_kind: Partial<Record<VisualKind, number>>;
  by_locale: Record<string, number>;
  pages_by_locale: Record<string, number>;
};
export type VisualReportMeta = { id: string; label: string | null; created_at: string; summary: VisualSummary };
export type VisualReport = VisualReportMeta & {
  report: {
    generated_at: string;
    options: { baseUrl: string; routes: string[]; locales: string[]; sourceLocale: string; urlPattern: string; viewports: string[] };
    pages: VisualPage[];
    issues: VisualIssue[];
  };
};
export type PreviewSettings = { base_url: string | null; url_pattern: string; routes: string[] };

// Live review — docs/design/live-review-contract.md (RV1). Prefix /api/projects/{slug}/review.
export type ReviewAnchor = {
  key?: string | null;
  key_source?: "attribute" | "marker" | "text" | "manual" | null;
  selector: string;
  text: string;
  url: string;
  route?: string;
  locale: string;
  viewport: { w: number; h: number; dpr?: number };
  /** Element box, CSS px relative to the document. */
  rect?: { x: number; y: number; w: number; h: number };
  /** Pin position inside the element, fractions 0..1. */
  point?: { x: number; y: number };
  dom?: string;
};
export type AnnotationStatus = "open" | "in_progress" | "resolved" | "wontfix";
export const ANNOTATION_STATUSES: AnnotationStatus[] = ["open", "in_progress", "resolved", "wontfix"];
export type ReviewActor = { user_id: string | null; link_id: string | null; name: string };
export type ReviewEmoji = "+1" | "-1" | "heart" | "eyes" | "check" | "approve";
export type ReviewReaction = { emoji: ReviewEmoji; count: number; mine: boolean };
export type ReviewComment = {
  id: string;
  annotation_id: string;
  author: ReviewActor;
  body: string;
  body_html: string;
  mentions: string[];
  created_at: string;
  edited_at: string | null;
};
export type Annotation = {
  id: string;
  status: AnnotationStatus;
  anchor: ReviewAnchor;
  route: string;
  locale: string;
  url: string;
  key: string | null;
  source: string | null;
  translation: { value: string; status: string } | null;
  body: string;
  /** Server-sanitised HTML: the only form of `body` ever rendered as HTML. */
  body_html: string;
  suggestion: string | null;
  author: ReviewActor;
  assignee: { user_id: string; name: string } | null;
  feedback_group_id: string | null;
  comment_count: number;
  attachments: { id: string; content_type: string; size: number; width: number | null; height: number | null; created_at: string }[];
  reactions: ReviewReaction[];
  approvals: number;
  created_at: string;
  updated_at: string;
  resolved_at: string | null;
  comments?: ReviewComment[];
};
export type NewAnnotation = { anchor: ReviewAnchor; body: string; suggestion?: string; assignee_id?: string };
export type AnnotationPatch = { status?: AnnotationStatus; assignee_id?: string | null; body?: string; suggestion?: string | null };
export type AnnotationQuery = { route?: string; locale?: string; status?: string; key?: string; author?: string; assignee?: string; q?: string; page?: number; per_page?: number };
export type ReviewEvent = { id: number; annotation_id: string | null; comment_id: string | null; kind: string; actor: ReviewActor; data: Record<string, unknown>; created_at: string };
export type ReviewAccess = "read" | "comment" | "edit";
export type ReviewLink = {
  id: string;
  label: string | null;
  access: ReviewAccess;
  created_at: string;
  expires_at: string | null;
  revoked_at: string | null;
  last_used_at: string | null;
  created_by: { user_id: string; name: string };
};
export type ReviewProbe = {
  url: string;
  status: number | null;
  error: "private" | "unreachable" | "timeout" | null;
  x_frame_options: string | null;
  frame_ancestors: string | null;
  framing: "allowed" | "blocked" | "unknown";
  widget: boolean;
  widget_project: string | null;
  widget_app_origin: string | null;
  feedback_widget: boolean;
  /** How the site switches language (hreflang alternates); null when the page wasn't fetched. */
  locale_scheme: {
    kind: "query" | "prefix" | "subdomain" | "unknown";
    pattern: string | null;
    locales: string[];
    default_locale: string | null;
    default_unprefixed: boolean;
    html_lang: string | null;
  } | null;
};
export type ReviewMember = { user_id: string; name: string };
/** RV4 (contract changelog v1.2): AI rewordings of an annotated text, each validated in every target locale. */
export type RephraseProposal = {
  id: string;
  text: string;
  explanation: string;
  valid: boolean;
  variants: Record<string, { value: string; back_translation: string | null; issues: Issue[]; policy?: unknown; valid: boolean }>;
};
export type Rephrase = {
  rewrite_id: string;
  cached: boolean;
  created_at: string;
  key: string | null;
  locale: string;
  proposals: RephraseProposal[];
  rejected: { text: string; reason: string }[];
  model?: string;
};
/** §6 manual editing. */
export type TextEntry = {
  key: string;
  source: string;
  source_locale: string;
  description: string | null;
  ui_role: string | null;
  values: Record<string, Translation>;
};
export type TextResults = Record<string, { issues: Issue[]; translation: Translation | null }>;
export type TextHistoryEntry = { id: string; locale: string; value: string; origin: string; model: string | null; author: string | null; created_at: string };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    /** `error.details` when the server sends it (e.g. per-locale validation results). */
    readonly details?: unknown,
  ) {
    super(message);
  }
}

/** Layout RUM (`@glot/insights/layout`): overflow per key × locale × viewport band, and the repairs proposed. */
export type LayoutRow = {
  key: string;
  locale: string;
  vw_min: number;
  vw_max: number | null;
  views: number;
  overflows: number;
  fallback: number;
  rate: number;
  box_px: number;
  need_px: number;
  font: string;
  current: string | null;
};
export type LayoutFix = {
  id: string;
  key: string;
  locale: string;
  previous: string;
  proposal: string | null;
  evidence: string;
  status: "pending" | "pr" | "suggested" | "failed";
  pr_url: string | null;
  error: string | null;
  created_at: string;
};
export type LayoutRum = { days: number; min_rate: number; min_views: number; rows: LayoutRow[]; fixes: LayoutFix[] };

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      credentials: "same-origin",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network", "Network error");
  }
  if (!res.ok) {
    const err = (await res.json().catch(() => null)) as { error?: { code: string; message: string; details?: unknown } } | null;
    throw new ApiError(res.status, err?.error?.code ?? "http_error", err?.error?.message ?? res.statusText, err?.error?.details);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

const enc = encodeURIComponent;
const p = (slug: string) => `/api/projects/${enc(slug)}`;
const rv = (slug: string) => `${p(slug)}/review`;

function qs(params: Record<string, string | number | undefined>) {
  const s = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") s.set(k, String(v));
  const out = s.toString();
  return out ? `?${out}` : "";
}

export type InsightArm = { visits: number; conversions: number; rate: number };
export type InsightVerdict = "collecting" | "keep" | "remove" | "inconclusive";
export type InsightExperiment = {
  id: string;
  locale: string;
  traffic_pct: number;
  goal: string;
  started_at: string;
  ends_at: string;
  status: "running" | "kept" | "removed";
  decided_at: string | null;
  test: InsightArm;
  control: InsightArm;
  diff: number;
  ci_low: number;
  ci_high: number;
  required_per_arm: number | null;
  verdict: InsightVerdict;
};
export type InsightDemand = {
  language: string;
  visits: number;
  share: number;
  offered: boolean;
  conversions: number;
  conversion_rate: number | null;
  est_extra_conversions: number | null;
  est_cost_usd: number | null;
  top_countries: [string, number][];
};
export type InsightFunnel = {
  locale: string;
  is_source: boolean;
  visits: number;
  conversions: number;
  rate: number;
  ci_low: number;
  ci_high: number;
};
/** `GET /markets`: "which language next?" (Labs). */
export type Market = {
  locale: string;
  name: string;
  score: number;
  best: boolean;
  visits: number;
  share: number;
  conversion_rate: number | null;
  est_extra_conversions: number | null;
  internet_users_m: number;
  cost_usd: number;
  words: number;
  review_hours: number;
  rtl: boolean;
  font: string | null;
  plural_forms: number;
  effort: number;
  effort_notes: string[];
};
export type Markets = { days: number; total_visits: number; keys: number; words: number; model: string; data_as_of: string; markets: Market[] };

export type Insights = {
  public_key: string;
  days: number;
  goal: string;
  source_locale: string;
  offered: string[];
  keys: number;
  events: string[];
  total_visits: number;
  demand: InsightDemand[];
  funnel: InsightFunnel[];
  daily: { day: string; visits: number; unserved: number }[];
  experiments: InsightExperiment[];
};

export type FeedbackKind = "wrong" | "awkward" | "unclear";
export type FeedbackStatus = "open" | "proposed" | "accepted" | "rejected";
export type FeedbackReport = { kind: FeedbackKind; text: string; suggestion: string | null; url: string | null; dom: string | null; created_at: string };
export type FeedbackGroup = {
  id: string;
  key: string;
  locale: string;
  source: string;
  current: string | null;
  status: FeedbackStatus;
  count: number;
  proposal: string | null;
  previous: string | null;
  back_translation: string | null;
  verdict: Verdict | null;
  explanation: string | null;
  issues: Issue[];
  pr_url: string | null;
  updated_at: string;
  reports: FeedbackReport[];
};
export type FeedbackSettings = {
  public_key: string;
  threshold: number;
  github_repo: string | null;
  github_base: string;
  catalog_path: string;
  lock_path: string;
  has_github_token: boolean;
};
/** github_token: write-only, "" clears, absent keeps. github_repo: "" clears. */
export type FeedbackSettingsInput = Partial<Omit<FeedbackSettings, "public_key" | "has_github_token">> & { github_token?: string };
// Copy experiments (variants.rs): arm 0 = official translation.
export type VariantArm = {
  arm: number;
  value: string;
  tone: string;
  is_control: boolean;
  views: number;
  conversions: number;
  rate: number;
  alpha: number;
  beta: number;
  p_best: number;
  p_beats_control: number;
  lift: number;
};
export type VariantExperiment = {
  id: string;
  key: string;
  locale: string;
  source: string;
  source_changed: boolean;
  goal: string;
  explore_pct: number;
  min_n: number;
  status: "running" | "decided" | "promoted" | "stopped";
  winner: number | null;
  created_at: string;
  decided_at: string | null;
  arms: VariantArm[];
};
export type NewVariantExperiment = { key: string; locale: string; variants?: number; explore_pct?: number; min_n?: number; goal?: string };

// Segmented experiments (/api/projects/{slug}/xp) — docs/design/segmented-copy-experiments-contract.md §3–4.
export type XpStatus = "draft" | "running" | "paused" | "stopped" | "promoted" | "personalized";
export type XpPredicate = { attr: string; eq?: string; in?: string[]; not_in?: string[] };
export type XpSegmentRule = { all: XpPredicate[] };
export type XpSegment = { id: string; name: string; rule: XpSegmentRule };
/** values[locale][key] = text. Arm 0 = official catalog, never carries values. */
export type XpVariant = { arm: number; intent: string; approved: boolean; values: Record<string, Record<string, string>> };
export type XpFit = { max_width_px?: number; max_lines?: number; [k: string]: unknown };
export type XpExperiment = {
  id: string;
  name: string;
  kind: "copy" | "holdout";
  status: XpStatus;
  keys: { key: string; source: string; source_changed: boolean }[];
  locales: string[];
  cells: XpSegment[];
  allocation: "fixed" | "bandit" | "segmented";
  weights: number[] | null;
  traffic_pct: number;
  min_n: number;
  goal: string;
  metric: "conversion" | "revenue";
  currency: string | null;
  guardrails: string[];
  facts: string[];
  fits: Record<string, XpFit>;
  review: boolean;
  arms: XpVariant[];
  state: { cell: string; weights: number[]; looks: number; winner: number | null; personalized: boolean }[];
  created_at: string;
  started_at: string | null;
  ended_at: string | null;
};
export type NewXpExperiment = Partial<
  Pick<XpExperiment, "name" | "locales" | "allocation" | "weights" | "traffic_pct" | "min_n" | "goal" | "metric" | "currency" | "guardrails" | "facts" | "fits" | "review">
> & {
  keys?: string[];
  prefix?: string | null;
  /** Existing segment id, or an inline segment (created). */
  cells?: (string | { name: string; rule: XpSegmentRule })[];
  arms?: { intent: string; values: XpVariant["values"] }[];
};
export type XpCellCheck = { arm: number; key: string; locale: string; value: string | null; ok: boolean; issues: Issue[] };
export type XpReadiness = { ready: boolean; problems: string[]; cells: XpCellCheck[] };
export type XpSettings = { holdout_pct: number; persistence: "none" | "session"; holdout_id: string | null };
// ponytail: estimate (XP2) and generate (XP8) bodies aren't pinned in the contract v1; typed as the wizard reads them.
export type XpEstimate = {
  cells: { cell: string; name?: string; visitors_per_day: number; days: number | null; too_small?: boolean }[];
  days?: number | null;
};
export type XpGenerated = {
  arms: XpVariant[];
  rejected: { intent?: string; text?: string; reason?: string; code?: string }[];
  policy?: { arm?: number; level?: "warn" | "block"; message: string }[];
};

// Results (XP2, experiments/stats.rs `Analysis`). ArmResult index = arm. ponytail: views/conversions per arm are
// optional until /results sends them; the UI shows them when present.
export type XpArmResult = {
  /** Posterior mean of the metric: conversion rate, or revenue per visitor. */
  mean: number;
  p_best: number;
  p_beats_control: number;
  lift: number;
  lift_lo: number;
  lift_hi: number;
  weight: number;
  views?: number;
  conversions?: number;
};
/** `name` = cell id: a segment uuid or "*". */
export type XpCellResult = {
  name: string;
  views: number;
  arms: XpArmResult[];
  winner: number | null;
  p_specific: number | null;
  srm_p: number | null;
  days_left?: number | null;
};
export type XpResults = {
  look: number;
  threshold: number;
  global: XpCellResult;
  cells: XpCellResult[];
  holdout_srm_p: number | null;
  paused: boolean;
  everywhere: number | null;
  segments: [string, number][];
  /** Days since start (falls back to started_at). */
  days?: number;
};
export type XpAction =
  | { action: "pause" | "resume" | "stop" }
  | { action: "promote"; arm: number; scope: "all" }
  | { action: "promote"; arm: number; scope: "segment"; cell: string };

// Billing (billing.rs). Limits/usage keys are shared; null limit = unlimited.
export type QuotaKey = "projects" | "seats" | "cdn_requests" | "insights_events" | "mcp_calls" | "ondemand_jobs" | "llm_strings";
export type Limits = Partial<Record<QuotaKey, number | null>>;
export type PlanId = "free" | "pro" | "team";
export type BillingSummary = {
  enabled: boolean;
  plan: PlanId | "self_hosted";
  subscribed_plan: string;
  status: string;
  current_period_end: string | null;
  cancel_at_period_end: boolean;
  grace_until: string | null;
  metered: boolean;
  has_customer: boolean;
  limits: Limits;
  usage: Partial<Record<QuotaKey, number>>;
};
export type Invoice = {
  id: string;
  number: string | null;
  status: string | null;
  total: number;
  currency: string;
  created: string | null;
  hosted_invoice_url: string | null;
  invoice_pdf: string | null;
};
export type PlanPrice = { unit_amount: number | null; unit_amount_decimal: string | null; currency: string; interval: string | null };
export type PlansInfo = { enabled: boolean; plans: Partial<Record<PlanId, Limits>>; prices: Record<string, PlanPrice> };

export const api = {
  signup: (b: { email: string; password: string; name: string; locale?: string }) =>
    request<User>("POST", "/api/auth/signup", b),
  login: (b: { email: string; password: string }) => request<User>("POST", "/api/auth/login", b),
  logout: () => request<void>("POST", "/api/auth/logout"),
  me: () => request<Me>("GET", "/api/me"),
  updateMe: (b: { locale?: string; notify_job_done?: boolean }) => request<Me>("PATCH", "/api/me", b),
  verifyEmail: (token: string) => request<void>("POST", "/api/auth/verify-email", { token }),
  resendVerification: () => request<void>("POST", "/api/auth/verify-email/resend"),
  forgotPassword: (email: string) => request<void>("POST", "/api/auth/forgot-password", { email }),
  resetPassword: (b: { token: string; password: string }) => request<void>("POST", "/api/auth/reset-password", b),
  changePassword: (b: { current_password: string; password: string }) => request<void>("PUT", "/api/me/password", b),
  changeEmail: (b: { email: string; password: string }) => request<{ pending: boolean }>("PUT", "/api/me/email", b),

  tokens: () => request<ApiToken[]>("GET", "/api/tokens"),
  createToken: (name: string) => request<{ token: string; api_token: ApiToken }>("POST", "/api/tokens", { name }),
  deleteToken: (id: string) => request<void>("DELETE", `/api/tokens/${enc(id)}`),

  projects: () => request<Project[]>("GET", "/api/projects"),
  createProject: (b: { name: string; slug: string; source_locale: string; target_locales: string[] }) =>
    request<Project>("POST", "/api/projects", b),
  project: (slug: string) => request<Project>("GET", p(slug)),
  updateProject: (slug: string, b: ProjectPatch) => request<Project>("PATCH", p(slug), b),
  deleteProject: (slug: string) => request<void>("DELETE", p(slug)),

  keys: (slug: string, q: KeyQuery) => request<KeyPage>("GET", `${p(slug)}/keys${qs(q)}`),
  putTranslation: (slug: string, key: string, locale: string, value: string) =>
    request<Translation>("PUT", `${p(slug)}/keys/${enc(key)}/translations/${enc(locale)}`, { value }),
  review: (slug: string, key: string, locale: string) =>
    request<Translation>("POST", `${p(slug)}/keys/${enc(key)}/translations/${enc(locale)}/review`),
  history: (slug: string, key: string) => request<HistoryEntry[]>("GET", `${p(slug)}/keys/${enc(key)}/history`),

  glossary: (slug: string) => request<Term[]>("GET", `${p(slug)}/glossary`),
  createTerm: (slug: string, b: TermInput) => request<Term>("POST", `${p(slug)}/glossary`, b),
  updateTerm: (slug: string, id: string, b: Partial<TermInput>) =>
    request<Term>("PATCH", `${p(slug)}/glossary/${enc(id)}`, b),
  deleteTerm: (slug: string, id: string) => request<void>("DELETE", `${p(slug)}/glossary/${enc(id)}`),

  issues: (slug: string) => request<ProjectIssue[]>("GET", `${p(slug)}/issues`),

  jobs: (slug: string) => request<Job[]>("GET", `${p(slug)}/jobs`),
  createJob: (slug: string, b: { kind: "translate"; locales?: string[]; keys?: string[]; force?: boolean }) =>
    request<Job>("POST", `${p(slug)}/jobs`, b),
  job: (id: string) => request<Job>("GET", `/api/jobs/${enc(id)}`),
  jobEventsUrl: (id: string) => `/api/jobs/${enc(id)}/events`,
  reviewLocale: (slug: string, b: { locale: string; keys?: string[]; refresh?: boolean }) =>
    request<Review[]>("POST", `${p(slug)}/review`, b),
  reviewKey: (slug: string, key: string, locale: string) =>
    request<Review>("GET", `${p(slug)}/keys/${enc(key)}/review${qs({ locale })}`),

  visualReports: (slug: string) => request<VisualReportMeta[]>("GET", `${p(slug)}/visual-reports`),
  visualReport: (slug: string, id: string | "latest") => request<VisualReport>("GET", `${p(slug)}/visual-reports/${enc(id)}`),
  visualFileUrl: (slug: string, id: string, path: string) =>
    `${p(slug)}/visual-reports/${enc(id)}/files/${path.split("/").map(enc).join("/")}`,
  preview: (slug: string) => request<PreviewSettings>("GET", `${p(slug)}/preview`),
  updatePreview: (slug: string, b: PreviewSettings) => request<PreviewSettings>("PUT", `${p(slug)}/preview`, b),
  insights: (slug: string, q: { days?: number; goal?: string }) => request<Insights>("GET", `${p(slug)}/insights${qs(q)}`),
  markets: (slug: string, days = 30) => request<Markets>("GET", `${p(slug)}/markets${qs({ days })}`),
  rotateInsightsKey: (slug: string) => request<{ public_key: string }>("POST", `${p(slug)}/insights/rotate-key`),
  createExperiment: (slug: string, b: { locale: string; traffic_pct: number; days: number; goal?: string }) =>
    request<InsightExperiment>("POST", `${p(slug)}/insights/experiments`, b),
  decideExperiment: (slug: string, id: string, status: "kept" | "removed") =>
    request<void>("PATCH", `${p(slug)}/insights/experiments/${enc(id)}`, { status }),
  layoutRum: (slug: string, days?: number) => request<LayoutRum>("GET", `${p(slug)}/insights/layout${qs({ days })}`),
  repairLayout: (slug: string) => request<{ created: number; fixes: LayoutFix[] }>("POST", `${p(slug)}/insights/layout/repair`),
  feedback: (slug: string) => request<{ settings: FeedbackSettings; groups: FeedbackGroup[] }>("GET", `${p(slug)}/feedback`),
  updateFeedbackSettings: (slug: string, b: FeedbackSettingsInput) => request<FeedbackSettings>("PUT", `${p(slug)}/feedback/settings`, b),
  feedbackAction: (slug: string, id: string, action: "propose" | "accept" | "reject" | "pr", b?: { value?: string }) =>
    request<FeedbackGroup>("POST", `${p(slug)}/feedback/${enc(id)}/${action}`, b),
  annotations: (slug: string, q: AnnotationQuery) => request<{ items: Annotation[]; total: number }>("GET", `${rv(slug)}/annotations${qs(q)}`),
  annotation: (slug: string, id: string) => request<Annotation>("GET", `${rv(slug)}/annotations/${enc(id)}`),
  createAnnotation: (slug: string, b: NewAnnotation) => request<Annotation>("POST", `${rv(slug)}/annotations`, b),
  updateAnnotation: (slug: string, id: string, b: AnnotationPatch) => request<Annotation>("PATCH", `${rv(slug)}/annotations/${enc(id)}`, b),
  deleteAnnotation: (slug: string, id: string) => request<void>("DELETE", `${rv(slug)}/annotations/${enc(id)}`),
  commentAnnotation: (slug: string, id: string, b: { body: string; mentions?: string[] }) =>
    request<ReviewComment>("POST", `${rv(slug)}/annotations/${enc(id)}/comments`, b),
  updateComment: (slug: string, id: string, cid: string, b: { body: string }) =>
    request<ReviewComment>("PATCH", `${rv(slug)}/annotations/${enc(id)}/comments/${enc(cid)}`, b),
  deleteComment: (slug: string, id: string, cid: string) => request<void>("DELETE", `${rv(slug)}/annotations/${enc(id)}/comments/${enc(cid)}`),
  reactAnnotation: (slug: string, id: string, emoji: ReviewEmoji) =>
    request<{ reactions: ReviewReaction[] }>("POST", `${rv(slug)}/annotations/${enc(id)}/reactions`, { emoji }),
  /** RV4: AI rewordings validated in every locale; the first valid one becomes the annotation's `suggestion`. */
  rephraseAnnotation: (slug: string, id: string, b: { instruction?: string }) =>
    request<Rephrase>("POST", `${rv(slug)}/annotations/${enc(id)}/rephrase`, b),
  /** RV4: writes every locale of a proposal (atomic) and resolves the annotation. */
  applyRephrase: (slug: string, id: string, b: { rewrite_id: string; proposal_id: string }) =>
    request<{ annotation: Annotation; results: TextResults; source_text: string }>("POST", `${rv(slug)}/annotations/${enc(id)}/rephrase/apply`, b),
  /** Writes `value` (default: the suggestion) as the reviewed translation and resolves the annotation. */
  applyAnnotation: (slug: string, id: string, b: { value?: string }) => request<Annotation>("POST", `${rv(slug)}/annotations/${enc(id)}/apply`, b),
  annotationFromFeedback: (slug: string, groupId: string) => request<Annotation>("POST", `${rv(slug)}/from-feedback/${enc(groupId)}`),
  reviewMembers: (slug: string) => request<ReviewMember[]>("GET", `${rv(slug)}/members`),
  reviewSession: (slug: string) => request<{ token: string; expires_at: string }>("POST", `${rv(slug)}/session`),
  /** Server-side check of the project's site URL: framing headers + review widget tag (Review page setup). */
  reviewProbe: (slug: string) => request<ReviewProbe>("GET", `${rv(slug)}/probe`),
  reviewEventsUrl: (slug: string) => `${rv(slug)}/events`,
  reviewLinks: (slug: string) => request<ReviewLink[]>("GET", `${rv(slug)}/links`),
  createReviewLink: (slug: string, b: { label?: string; access: ReviewAccess; expires_in_days: number | null }) =>
    request<{ token: string; link: ReviewLink }>("POST", `${rv(slug)}/links`, b),
  deleteReviewLink: (slug: string, id: string) => request<void>("DELETE", `${rv(slug)}/links/${enc(id)}`),
  text: (slug: string, key: string) => request<TextEntry>("GET", `${rv(slug)}/text/${enc(key)}`),
  putText: (slug: string, key: string, b: { values: Record<string, string>; dry_run?: boolean }) =>
    request<{ results: TextResults }>("PUT", `${rv(slug)}/text/${enc(key)}`, b),
  textHistory: (slug: string, key: string, locale?: string) => request<TextHistoryEntry[]>("GET", `${rv(slug)}/text/${enc(key)}/history${qs({ locale })}`),
  revertText: (slug: string, key: string, b: { locale: string; history_id?: string }) =>
    request<{ translation: Translation }>("POST", `${rv(slug)}/text/${enc(key)}/revert`, b),
  deleteText: (slug: string, key: string, locale: string) => request<void>("DELETE", `${rv(slug)}/text/${enc(key)}/translations/${enc(locale)}`),
  createText: (slug: string, b: { source: string; key?: string; description?: string; anchor?: ReviewAnchor; values?: Record<string, string> }) =>
    request<TextEntry>("POST", `${rv(slug)}/text`, b),
  variantExperiments: (slug: string) => request<VariantExperiment[]>("GET", `${p(slug)}/experiments`),
  createVariantExperiment: (slug: string, b: NewVariantExperiment) =>
    request<VariantExperiment & { rejected: { text: string; tone: string }[] }>("POST", `${p(slug)}/experiments`, b),
  variantAction: (slug: string, id: string, action: "promote" | "stop") =>
    request<unknown>("POST", `${p(slug)}/experiments/${enc(id)}/${action}`),

  xpList: (slug: string) => request<XpExperiment[]>("GET", `${p(slug)}/xp`),
  xpCreate: (slug: string, b: NewXpExperiment) => request<XpExperiment>("POST", `${p(slug)}/xp`, b),
  xp: (slug: string, id: string) => request<XpExperiment>("GET", `${p(slug)}/xp/${enc(id)}`),
  xpUpdate: (slug: string, id: string, b: NewXpExperiment) => request<XpExperiment>("PATCH", `${p(slug)}/xp/${enc(id)}`, b),
  xpPutArm: (slug: string, id: string, arm: number, b: { intent?: string; approved?: boolean; values?: XpVariant["values"] }) =>
    request<XpVariant>("PUT", `${p(slug)}/xp/${enc(id)}/arms/${arm}`, b),
  xpDeleteArm: (slug: string, id: string, arm: number) => request<void>("DELETE", `${p(slug)}/xp/${enc(id)}/arms/${arm}`),
  xpCheck: (slug: string, id: string) => request<XpReadiness>("GET", `${p(slug)}/xp/${enc(id)}/check`),
  xpStart: (slug: string, id: string) => request<XpExperiment>("POST", `${p(slug)}/xp/${enc(id)}/start`),
  xpGenerate: (slug: string, id: string, b: { intents: string[]; locales?: string[] }) =>
    request<XpGenerated>("POST", `${p(slug)}/xp/${enc(id)}/generate`, b),
  xpEstimate: (slug: string, b: { cells: NewXpExperiment["cells"]; arms: number; locales?: string[]; goal?: string; min_n?: number }) =>
    request<XpEstimate>("POST", `${p(slug)}/xp/estimate`, b),
  xpResults: (slug: string, id: string) => request<XpResults>("GET", `${p(slug)}/xp/${enc(id)}/results`),
  xpAction: (slug: string, id: string, { action, ...body }: XpAction) =>
    request<XpExperiment>("POST", `${p(slug)}/xp/${enc(id)}/${action}`, action === "promote" ? body : undefined),
  xpSettings: (slug: string) => request<XpSettings>("GET", `${p(slug)}/xp/settings`),
  xpUpdateSettings: (slug: string, b: Partial<Pick<XpSettings, "holdout_pct" | "persistence">>) =>
    request<XpSettings>("PUT", `${p(slug)}/xp/settings`, b),
  billing: () => request<BillingSummary>("GET", "/api/billing"),
  billingPlans: () => request<PlansInfo>("GET", "/api/billing/plans"),
  invoices: () => request<Invoice[]>("GET", "/api/billing/invoices"),
  checkout: (b: { plan: "pro" | "team"; interval: "month" | "year" }) => request<{ url: string }>("POST", "/api/billing/checkout", b),
  billingPortal: () => request<{ url: string }>("POST", "/api/billing/portal"),
};

export const qk = {
  me: ["me"] as const,
  tokens: ["tokens"] as const,
  projects: ["projects"] as const,
  project: (slug: string) => ["project", slug] as const,
  keys: (slug: string) => ["keys", slug] as const,
  history: (slug: string, key: string) => ["history", slug, key] as const,
  glossary: (slug: string) => ["glossary", slug] as const,
  issues: (slug: string) => ["issues", slug] as const,
  jobs: (slug: string) => ["jobs", slug] as const,
  review: (slug: string, key: string, locale: string) => ["review", slug, key, locale] as const,
  visualReports: (slug: string) => ["visual-reports", slug] as const,
  visualReport: (slug: string, id: string) => ["visual-reports", slug, id] as const,
  preview: (slug: string) => ["preview", slug] as const,
  insights: (slug: string) => ["insights", slug] as const,
  markets: (slug: string) => ["markets", slug] as const,
  feedback: (slug: string) => ["feedback", slug] as const,
  annotations: (slug: string) => ["annotations", slug] as const,
  annotation: (slug: string, id: string) => ["annotations", slug, "one", id] as const,
  reviewMembers: (slug: string) => ["review-members", slug] as const,
  reviewLinks: (slug: string) => ["review-links", slug] as const,
  text: (slug: string, key: string) => ["review-text", slug, key] as const,
  variants: (slug: string) => ["variants", slug] as const,
  xp: (slug: string, id: string) => ["variants", slug, id] as const,
  xpCheck: (slug: string, id: string) => ["variants", slug, id, "check"] as const,
  xpResults: (slug: string, id: string) => ["variants", slug, id, "results"] as const,
  xpList: (slug: string) => ["variants", slug, "xp"] as const,
  billing: ["billing"] as const,
  invoices: ["invoices"] as const,
};
