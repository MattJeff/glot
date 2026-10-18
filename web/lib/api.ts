// Typed client for the glot-server HTTP API (see ARCHITECTURE.md § API HTTP).
// Requests go through the Next.js rewrite (/api → :8790), so the session cookie is same-origin.

export type User = { id: string; email: string; name: string };
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
};
export type ProjectPatch = {
  name?: string;
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

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

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
    const err = (await res.json().catch(() => null)) as { error?: { code: string; message: string } } | null;
    throw new ApiError(res.status, err?.error?.code ?? "http_error", err?.error?.message ?? res.statusText);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

const enc = encodeURIComponent;
const p = (slug: string) => `/api/projects/${enc(slug)}`;

function qs(params: Record<string, string | number | undefined>) {
  const s = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") s.set(k, String(v));
  const out = s.toString();
  return out ? `?${out}` : "";
}

export const api = {
  signup: (b: { email: string; password: string; name: string }) => request<User>("POST", "/api/auth/signup", b),
  login: (b: { email: string; password: string }) => request<User>("POST", "/api/auth/login", b),
  logout: () => request<void>("POST", "/api/auth/logout"),
  me: () => request<User>("GET", "/api/me"),

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
};
