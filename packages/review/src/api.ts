// Widget side of docs/design/live-review-contract.md (prefix W = /api/review/w, Bearer glr_…).
import type { ReviewOptions } from "./types";

export type Actor = { user_id: string | null; link_id: string | null; name: string };
export type Reaction = { emoji: string; count: number; mine: boolean };
export type Issue = { code?: string; severity: "error" | "warning"; message: string };
export type Anchor = {
  key?: string | null; key_source?: "attribute" | "marker" | "text" | "manual" | null;
  selector: string; text: string; url: string; locale: string;
  viewport: { w: number; h: number; dpr?: number };
  rect?: { x: number; y: number; w: number; h: number }; point?: { x: number; y: number }; dom?: string;
};
export type Comment = { id: string; author: Actor; body: string; body_html: string; created_at: string; edited_at: string | null };
export type Ann = {
  id: string; status: "open" | "in_progress" | "resolved" | "wontfix"; anchor: Anchor; route: string; locale: string;
  key: string | null; source: string | null; translation: { value: string; status: string } | null;
  body: string; body_html: string; suggestion: string | null; author: Actor; comment_count: number;
  attachments: { id: string }[]; reactions: Reaction[]; approvals: number; created_at: string; updated_at: string;
  comments?: Comment[];
};
export type Variant = { value: string; back_translation: string | null; issues: Issue[]; valid: boolean };
export type Proposal = { id: number; text: string; explanation: string; variants: Record<string, Variant>; valid: boolean };
export type Rephrased = { rewrite_id: string; proposals: Proposal[]; rejected: { text: string; reason: string[] }[] };
export type Translation = { value: string; status: string; issues: Issue[] };
export type TextEntry = { key: string; source: string; source_locale: string; description: string | null; values: Record<string, Translation> };
export type Context = { project: { slug: string; name: string; source_locale: string; target_locales: string[] }; access: "read" | "comment" | "edit" | "member"; me: Actor; expires_at: string | null };
export type ReviewEvent = { id: number; annotation_id: string | null; kind: string; actor: Actor; data: any };

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string, public details?: any) { super(message); }
}

/** `guard` runs before every write (the widget uses it to ask a review-link reviewer for their name). */
export function makeApi(o: ReviewOptions & { token: string }, name: () => string | undefined, guard: () => void = () => {}) {
  const base = (o.endpoint || "").replace(/\/$/, "") + "/api/review/w";
  async function call<T>(method: string, path: string, body?: unknown, raw?: Blob): Promise<T> {
    if (method !== "GET") guard();
    const r = await fetch(base + path, {
      method,
      headers: { authorization: `Bearer ${o.token}`, ...(raw ? { "content-type": raw.type } : body !== undefined ? { "content-type": "application/json" } : {}) },
      body: raw ?? (body === undefined ? undefined : JSON.stringify(body)),
    });
    if (r.status === 204) return undefined as T;
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new ApiError(r.status, j?.error?.code || "", j?.error?.message || r.statusText, j?.error?.details);
    return j;
  }
  // `author_name` is required for review-link writes and ignored for members.
  const w = <T extends object>(b: T) => ({ ...b, author_name: name() });
  const A = (id: string) => `/annotations/${id}`;
  // v1.3: bodiless writes (DELETE, capture upload) carry the link reviewer's name in the query.
  const nq = () => (name() ? "?author_name=" + encodeURIComponent(name()!) : "");
  const T = (key: string) => `/text/${encodeURIComponent(key)}`;
  return {
    context: () => call<Context>("GET", "/context"),
    // ponytail: first 100 annotations of the page; paginate if a single page ever collects more.
    list: (route: string, locale: string) =>
      call<{ items: Ann[] }>("GET", `/annotations?route=${encodeURIComponent(route)}&locale=${encodeURIComponent(locale)}&status=open,in_progress,resolved,wontfix&per_page=100`).then((r) => r.items),
    get: (id: string) => call<Ann>("GET", A(id)),
    create: (anchor: Anchor, body: string, suggestion?: string) => call<Ann>("POST", "/annotations", w({ anchor, body, suggestion })),
    patch: (id: string, b: Partial<{ status: string; body: string; suggestion: string | null }>) => call<Ann>("PATCH", A(id), w(b)),
    del: (id: string) => call<void>("DELETE", A(id) + nq()),
    comment: (id: string, body: string) => call<Comment>("POST", A(id) + "/comments", w({ body })),
    editComment: (id: string, cid: string, body: string) => call<Comment>("PATCH", `${A(id)}/comments/${cid}`, w({ body })),
    delComment: (id: string, cid: string) => call<void>("DELETE", `${A(id)}/comments/${cid}${nq()}`),
    react: (id: string, emoji: string, comment_id?: string) => call<{ reactions: Reaction[] }>("POST", A(id) + "/reactions", w({ emoji, comment_id })),
    attach: (id: string, img: Blob) => call<unknown>("POST", `${A(id)}/attachments${nq()}`, undefined, img),
    shot: (id: string, aid: string) => `${base}${A(id)}/attachments/${aid}?token=${encodeURIComponent(o.token)}`,
    apply: (id: string, value?: string) => call<Ann>("POST", A(id) + "/apply", w({ value })),
    /** RV4: 2-3 proposals, each declined and validated in every project locale. */
    rephrase: (id: string, preset: string, segment?: string) => call<Rephrased>("POST", A(id) + "/rephrase", w({ preset, segment, count: 3 })),
    applyRewrite: (id: string, rewrite_id: string, proposal_id: number) => call<unknown>("POST", A(id) + "/rephrase/apply", w({ rewrite_id, proposal_id })),
    text: (key: string) => call<TextEntry>("GET", T(key)),
    /** dry_run: live validation. Throws ApiError 422 (details.results) when a locale has an error. */
    put: (key: string, values: Record<string, string>, dry_run = false) =>
      call<{ results: Record<string, { issues: Issue[] }> }>("PUT", T(key), w({ values, dry_run })),
    revert: (key: string, locale: string) => call<{ translation: Translation }>("POST", T(key) + "/revert", w({ locale })),
    unset: (key: string, locale: string) => call<void>("DELETE", `${T(key)}/translations/${encodeURIComponent(locale)}${nq()}`),
    createKey: (source: string, anchor: Anchor, key?: string) => call<TextEntry>("POST", "/text", w({ source, key: key || undefined, anchor })),
    events: (on: (e: ReviewEvent) => void) => {
      const es = new EventSource(`${base}/events?token=${encodeURIComponent(o.token)}`);
      es.addEventListener("review", (m) => { try { on(JSON.parse((m as MessageEvent).data)); } catch {} });
      return es;
    },
  };
}
export type Api = ReturnType<typeof makeApi>;
