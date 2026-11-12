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
  /** `translate` = preview / paused (nothing translated or written), `push` = translated, published by hand. */
  hold: "translate" | "push" | null;
  last_sync_at: string | null;
  last_push_at: string | null;
  last_webhook_at: string | null;
  last_error: string | null;
  /** What to do about `last_error` (`connectors.fix.<code>`). */
  fix: string | null;
  reconnect: boolean;
  token_expires_at: string | null;
  stats: {
    items?: number;
    units?: number;
    pulled?: number;
    to_translate?: number;
    /** Fields per content type, excluded types included. */
    by_type?: Record<string, number>;
    /** Products, pages… per content type. */
    resources?: Record<string, number>;
    words?: number;
    cost_usd?: number;
    locales?: string[];
    last_push?: { pushed: number; unchanged: number; waiting: number; kept?: number };
  };
  issues: { item: string; error: string; fix: string | null }[];
  job: { id: string; status: string; done: number; total: number } | null;
  webhook_url: string;
  created_at: string;
};
export type Check = { id: string; status: "ok" | "warn" | "fail"; detail: string; fix: string | null };
export type Doctor = { ok: boolean; checks: Check[] };
export type DiffRow = {
  locale: string;
  id: string;
  field: string;
  source: string;
  value: string;
  update: boolean;
  blocked: string | null;
  fix: string | null;
};
export type Diff = { total: number; waiting: number; unchanged: number; rows: DiffRow[] };
export type Run = {
  id: string;
  started_at: string;
  stats: { pushed?: number; kept?: number; failed?: number };
  rolled_back_at: string | null;
};
export type Rollback = { restored: number; removed: number; left: number; kept: number };
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
    b: {
      kind: string;
      name?: string;
      secrets: Record<string, string>;
      settings?: Record<string, unknown>;
      /** First sync stops after reading the content and waits for an approval. */
      preview?: boolean;
    },
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
  /** Kinds with a one-click Connect (OAuth client configured on the server, or none needed). */
  oneClick: (slug: string) => call<string[]>("GET", `${p(slug)}/oauth`),
  /** Where to send the browser to approve; `account` = reconnect that one. */
  start: (slug: string, kind: string, b: { tenant?: string; account?: string }) =>
    call<{ url: string }>("POST", `${p(slug)}/oauth/${kind}/start`, b),
  hold: (slug: string, id: string, hold: Connector["hold"]) => call<Connector>("POST", `${p(slug)}/${id}/hold`, { hold }),
  publish: (slug: string, id: string) =>
    call<{ connector: Connector; error: string | null }>("POST", `${p(slug)}/${id}/publish`),
  doctor: (slug: string, id: string) => call<Doctor>("POST", `${p(slug)}/${id}/doctor`),
  diff: (slug: string, id: string) => call<Diff>("GET", `${p(slug)}/${id}/diff?limit=30`),
  runs: (slug: string, id: string) => call<Run[]>("GET", `${p(slug)}/${id}/runs`),
  rollback: (slug: string, id: string, run: string) =>
    call<Rollback>("POST", `${p(slug)}/${id}/runs/${run}/rollback`),
};
