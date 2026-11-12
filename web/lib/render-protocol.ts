// Render protocol (crates/server/src/render_protocol.rs): playground, keys and usage of the dashboard page.
import { request } from "@/lib/api";

export type RenderType = "text" | "html" | "markdown" | "template";
export type RenderResult = {
  id: string | null;
  locale: string;
  type: string;
  status: "translated" | "source" | "partial" | "fallback";
  content: unknown;
  cache: "hit" | "miss" | "partial" | "none";
  units: { total: number; cached: number; translated: number; fallback: number };
  fallback_reason?: string;
  warnings: string[];
  error?: { code: string; message: string };
};
export type RenderKey = {
  id: string;
  name: string;
  prefix: string;
  scopes: string[];
  monthly_limit: number | null;
  created_at: string;
  last_used_at: string | null;
  used_this_month: number;
};
export type RenderCounters = { requests: number; units: number; cached: number; translated: number; fallbacks: number; cost_usd: number };
export type RenderUsage = {
  days: (RenderCounters & { day: string })[];
  totals: RenderCounters;
  hit_rate: number | null;
  memory_entries: number;
  memory_locales: number;
};

const base = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/render`;

export const renderApi = {
  run: (slug: string, items: { locale: string; type: RenderType; content: string; variables?: Record<string, unknown> }[]) =>
    request<{ items: RenderResult[] }>("POST", base(slug), { items }),
  keys: (slug: string) => request<RenderKey[]>("GET", `${base(slug)}/keys`),
  createKey: (slug: string, b: { name: string; scopes: string[]; monthly_limit: number | null }) =>
    request<RenderKey & { key: string }>("POST", `${base(slug)}/keys`, b),
  deleteKey: (slug: string, id: string) => request<void>("DELETE", `${base(slug)}/keys/${encodeURIComponent(id)}`),
  usage: (slug: string) => request<RenderUsage>("GET", `${base(slug)}/usage`),
};

export const renderQk = {
  keys: (slug: string) => ["render-keys", slug] as const,
  usage: (slug: string) => ["render-usage", slug] as const,
};

/** The call a backend makes; `key` is shown only right after creation. */
export const renderCurl = (origin: string | undefined, key?: string) =>
  `curl ${origin ?? "https://glot.example.com"}/v1/render \\
  -H "Authorization: Bearer ${key ?? "glot_rk_…"}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "locale": "fr-FR",
    "type": "email",
    "content": { "subject": "…", "html": "<p>… {{first_name}} …</p>" },
    "variables": { "first_name": "…" }
  }'`;
