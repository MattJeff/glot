// CMS connectors API (crates/server/src/connectors/api.rs). Credentials go in, never come back:
// `secret_fields` lists which ones are set.
import { ApiError } from "@/lib/api";

export type ConnectorStatus = "idle" | "queued" | "running" | "ok" | "error";
export type Connector = {
  id: string;
  kind: string;
  name: string;
  settings: Record<string, unknown>;
  secret_fields: string[];
  status: ConnectorStatus;
  last_sync_at: string | null;
  last_push_at: string | null;
  last_error: string | null;
  stats: {
    items?: number;
    units?: number;
    to_translate?: number;
    last_push?: { pushed: number; unchanged: number; waiting: number };
  };
  webhook_url: string;
  created_at: string;
};
export type AccountInfo = { name: string; source_locale: string | null; notes: string[] };

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      credentials: "include",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network", "network error");
  }
  if (res.status === 204) return undefined as T;
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new ApiError(res.status, data?.error?.code ?? "error", data?.error?.message ?? res.statusText);
  return data as T;
}

const p = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/connectors`;
export const connectorKey = (slug: string) => ["connectors", slug] as const;
export const connectorsApi = {
  list: (slug: string) => call<Connector[]>("GET", p(slug)),
  create: (
    slug: string,
    b: { kind: string; name?: string; secrets: Record<string, string>; settings?: Record<string, unknown> },
  ) =>
    call<{ connector: Connector; info: AccountInfo; source_locale_mismatch: boolean; webhooks_error: string | null }>(
      "POST",
      p(slug),
      b,
    ),
  update: (slug: string, id: string, b: { name?: string; secrets?: Record<string, string>; settings?: Record<string, unknown> }) =>
    call<Connector>("PATCH", `${p(slug)}/${id}`, b),
  remove: (slug: string, id: string) => call<void>("DELETE", `${p(slug)}/${id}`),
  test: (slug: string, id: string) =>
    call<{ info: AccountInfo; locales: string[]; webhooks_error: string | null }>("POST", `${p(slug)}/${id}/test`),
  sync: (slug: string, id: string) => call<Connector>("POST", `${p(slug)}/${id}/sync`),
};
