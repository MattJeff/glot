// Client of the glot render API (`POST /v1/render`). Server-side only: the key is a secret.
//
// Fail-open: when glot cannot be reached (network, timeout, 5xx), `render` returns the source content with the
// variables filled locally and `status: "fallback"`, so the email still goes out. Refusals that need a fix
// (400 invalid request, 401 key, 402/429 quota) throw `GlotRenderError`.

export type TypedVariable =
  | { type: "number"; value: number; min_fraction?: number; max_fraction?: number }
  | { type: "percent"; value: number; max_fraction?: number }
  | { type: "currency"; value: number; currency: string }
  | { type: "date" | "datetime"; value: string | number; style?: "short" | "medium" | "long" };
export type Variables = { [name: string]: string | number | boolean | null | TypedVariable | Variables };

export type Email = { subject?: string; html?: string; text?: string };
type ContentOf = { text: string; html: string; markdown: string; template: string; email: Email; json: unknown };
export type RenderType = keyof ContentOf;

export type RenderRequest<T extends RenderType = RenderType> = {
  /** The recipient's locale: "fr", "pt-BR". */
  locale: string;
  type: T;
  content: ContentOf[T];
  /** Filled in by glot after translation, formatted for the locale. Never sent to a model, never stored. */
  variables?: Variables;
  /** `json` only: paths of the text fields, e.g. ["title", "items.*.name", { path: "body", type: "html" }]. */
  fields?: (string | { path: string; type?: "text" | "html" | "markdown" | "template" })[];
  /** What this content is ("password reset email"): guides new translations. */
  context?: string;
  allow_partial?: boolean;
  cache_only?: boolean;
  id?: string;
};

export type RenderResult<T extends RenderType = RenderType> = {
  id: string | null;
  locale: string;
  type: T;
  /** `fallback`: the content is in the source language; `fallback_reason` says why. */
  status: "translated" | "source" | "partial" | "fallback";
  content: ContentOf[T];
  cache: "hit" | "miss" | "partial" | "none";
  units: { total: number; cached: number; translated: number; fallback: number };
  fallback_reason?: string;
  warnings: string[];
  error?: { code: string; message: string };
};

export class GlotRenderError extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "GlotRenderError";
    this.status = status;
    this.code = code;
  }
}

export type Options = {
  /** Project render key (`glot_rk_…`). */
  key: string;
  /** Your glot server, e.g. "https://api.glotlang.com". */
  baseUrl: string;
  /** Client-side timeout; the server stops waiting for the model at the same time. Default 20 000. */
  timeoutMs?: number;
  /** Return the source instead of throwing when glot is unreachable. Default true. */
  failOpen?: boolean;
  fetch?: typeof fetch;
};

const escapeHtml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

function formatLocal(v: unknown, locale: string): string | undefined {
  if (v === null) return "";
  if (typeof v !== "object") return String(v);
  const t = v as TypedVariable;
  try {
    switch (t.type) {
      case "number":
        return new Intl.NumberFormat(locale, { minimumFractionDigits: t.min_fraction, maximumFractionDigits: t.max_fraction ?? 3 }).format(t.value);
      case "percent":
        return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: t.max_fraction ?? 0 }).format(t.value);
      case "currency":
        return new Intl.NumberFormat(locale, { style: "currency", currency: t.currency }).format(t.value);
      case "date":
      case "datetime": {
        const d = new Date(typeof t.value === "number" ? t.value * 1000 : t.value);
        return new Intl.DateTimeFormat(locale, {
          dateStyle: t.style ?? "medium",
          timeStyle: t.type === "datetime" ? "short" : undefined,
          timeZone: "UTC",
        }).format(d);
      }
    }
  } catch {
    /* unknown locale or currency: leave the token */
  }
  return undefined;
}

/** `{{name}}` / `{{{name}}}` filled with `Intl`; unknown names stay. Used only by the fail-open path. */
export function fillLocal(s: string, variables: Variables, locale: string, html: boolean): string {
  return s.replace(/\{\{(\{?)\s*([\w.-]+)\s*\}?\}\}/g, (token, raw: string, name: string) => {
    let v: unknown = variables[name];
    if (v === undefined) v = name.split(".").reduce<unknown>((o, k) => (o && typeof o === "object" ? (o as Record<string, unknown>)[k] : undefined), variables);
    const out = v === undefined ? undefined : formatLocal(v, locale);
    return out === undefined ? token : html && !raw ? escapeHtml(out) : out;
  });
}

function sourceFallback<T extends RenderType>(req: RenderRequest<T>, reason: string): RenderResult<T> {
  const v = req.variables;
  const fill = (s: string, html = false) => (v ? fillLocal(s, v, req.locale, html) : s);
  let content: unknown = req.content;
  if (req.type === "email") {
    const e = req.content as Email;
    content = { ...e, ...(e.subject != null && { subject: fill(e.subject) }), ...(e.html != null && { html: fill(e.html, true) }), ...(e.text != null && { text: fill(e.text) }) };
  } else if (req.type === "text" || req.type === "markdown" || req.type === "html") {
    content = fill(req.content as string, req.type === "html");
  }
  // `template` (ICU) and `json` are returned as sent: formatting them needs the server.
  return {
    id: req.id ?? null,
    locale: req.locale,
    type: req.type,
    status: "fallback",
    content: content as ContentOf[T],
    cache: "none",
    units: { total: 0, cached: 0, translated: 0, fallback: 0 },
    fallback_reason: reason,
    warnings: [],
  };
}

export function createClient(o: Options) {
  const f = o.fetch ?? fetch;
  const timeoutMs = o.timeoutMs ?? 20_000;
  const post = async (path: string, body: unknown, idempotencyKey?: string) => {
    const res = await f(`${o.baseUrl.replace(/\/$/, "")}${path}`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${o.key}`,
        "content-type": "application/json",
        ...(idempotencyKey && { "idempotency-key": idempotencyKey }),
      },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs + 2_000),
    });
    if (res.ok) return res.json();
    const err = (await res.json().catch(() => null)) as { error?: { code?: string; message?: string } } | null;
    throw new GlotRenderError(res.status, err?.error?.code ?? "http_error", err?.error?.message ?? res.statusText);
  };
  const open = (e: unknown) => {
    if (o.failOpen === false || (e instanceof GlotRenderError && e.status < 500)) throw e;
    return `client_fallback: glot was not reachable (${e instanceof Error ? e.message : String(e)})`;
  };
  return {
    /** Localize one piece of content. Never throws for an unreachable server unless `failOpen: false`. */
    async render<T extends RenderType>(req: RenderRequest<T>, opts?: { idempotencyKey?: string }): Promise<RenderResult<T>> {
      try {
        return (await post("/v1/render", { ...req, timeout_ms: timeoutMs }, opts?.idempotencyKey)) as RenderResult<T>;
      } catch (e) {
        return sourceFallback(req, open(e));
      }
    },
    /** Up to 50 items in one call (same content in N languages, or N contents). */
    async renderBatch(items: RenderRequest[], opts?: { idempotencyKey?: string }): Promise<RenderResult[]> {
      try {
        const r = (await post("/v1/render/batch", { items, timeout_ms: timeoutMs }, opts?.idempotencyKey)) as { items: RenderResult[] };
        return r.items;
      } catch (e) {
        const reason = open(e);
        return items.map((i) => sourceFallback(i, reason));
      }
    },
  };
}
