"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Play, Radio, Square, TriangleAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { CodeBlock, EmptyState, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Link } from "@/i18n/navigation";
import { request } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";
import { translateIterable, type LiveEvent } from "@/lib/live-stream";

type Settings = { enabled: boolean; browser: boolean; mask_pii: boolean; store: boolean; model: string | null; daily_chars: number };
type Day = { day: string; requests: number; chars: number; units: number; llm_units: number; known_units: number; failed_units: number; first_ms: number };
type Status = {
  settings: Settings;
  public_key: string;
  provider: { name: string; model: string; ready: boolean; error: string | null };
  glossary_terms: number;
  units: { distinct: number; repeats: number; stored: number };
  days: Day[];
};

/** One finished piece of the output, as measured in this browser. */
type Piece = { text: string; via: string; reason?: string };
type Run = {
  pieces: Piece[];
  /** ms between the first source character and the first translated text. */
  first: number | null;
  /** ms each translated piece appeared after its last source character had arrived. */
  lags: number[];
  fallback: string | null;
  done: boolean;
};

const SPEEDS = [40, 150, 0] as const;

export default function LiveStreamPage() {
  const t = useTranslations("liveStream");
  const slug = useSlug();
  const qc = useQueryClient();
  const localeName = useLocaleName();
  const project = useProject(slug);
  const key = ["live-stream", slug] as const;
  const path = `/api/projects/${slug}/live-stream`;
  const status = useQuery({ queryKey: key, queryFn: () => request<Status>("GET", path) });
  const save = useMutation({
    mutationFn: (s: Settings) => request<Settings>("PUT", path, s),
    onSuccess: () => qc.invalidateQueries({ queryKey: key }),
    onError: (e) => toast.error(e.message),
  });

  const [text, setText] = useState("");
  const [to, setTo] = useState("");
  const [speed, setSpeed] = useState<(typeof SPEEDS)[number]>(40);
  const [source, setSource] = useState("");
  const [run, setRun] = useState<Run | null>(null);
  const stop = useRef<AbortController | null>(null);

  if (status.isPending || project.isPending) return <Skeleton className="mx-auto h-96 max-w-5xl rounded-xl" />;
  if (status.isError) return <ErrorState error={status.error} onRetry={() => status.refetch()} />;
  const s = status.data;
  const locales = project.data?.target_locales ?? [];
  const target = to || locales[0] || "";
  const set = (patch: Partial<Settings>) => save.mutate({ ...s.settings, ...patch });

  async function start() {
    const ac = new AbortController();
    stop.current = ac;
    const arrivals: [number, number][] = []; // [source length so far, time]
    const state: Run = { pieces: [], first: null, lags: [], fallback: null, done: false };
    let t0 = 0;
    let cursor = 0;
    setSource("");
    setRun({ ...state });
    // The text you typed, replayed at the chosen pace: the pipeline cannot tell it from an answer being generated.
    const replay = (async function* () {
      const step = speed === 0 ? text.length : Math.max(1, Math.round(speed / 20));
      for (let i = 0; i < text.length && !ac.signal.aborted; i += step) {
        const chunk = text.slice(i, i + step);
        if (!t0) t0 = performance.now();
        arrivals.push([i + chunk.length, performance.now()]);
        setSource((v) => v + chunk);
        yield chunk;
        if (speed) await new Promise((r) => setTimeout(r, 50));
      }
    })();
    const onEvent = (e: LiveEvent) => {
      if (e.type === "fallback") {
        state.fallback = e.reason;
        return;
      }
      cursor += e.n;
      state.pieces.push({ text: e.text, via: e.via, reason: e.reason });
      if (e.via === "raw" || e.via === "source") return;
      const now = performance.now();
      const arrived = arrivals.find((a) => a[0] >= cursor)?.[1] ?? now;
      state.lags.push(now - arrived);
      state.first ??= now - t0;
    };
    try {
      let shown = 0;
      for await (const out of translateIterable(replay, {
        project: slug,
        to: target,
        from: project.data?.source_locale,
        url: `${path}/translate`,
        conversation: `playground-${Date.now()}`,
        signal: ac.signal,
        onEvent,
      })) {
        // After a fallback the helper yields source text without an event.
        shown += out.length;
        if (state.fallback && state.pieces.reduce((n, p) => n + p.text.length, 0) < shown) {
          state.pieces.push({ text: out, via: "source", reason: state.fallback });
        }
        setRun({ ...state, pieces: [...state.pieces] });
      }
    } finally {
      state.done = true;
      setRun({ ...state, pieces: [...state.pieces] });
      stop.current = null;
      void qc.invalidateQueries({ queryKey: key });
    }
  }

  const running = !!run && !run.done;
  const count = (via: string[]) => run?.pieces.filter((p) => via.includes(p.via)).length ?? 0;
  const reasons = [...new Set(run?.pieces.map((p) => p.reason).filter(Boolean) as string[])];
  const median = (v: number[]) => (v.length ? [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)] : null);
  const ms = (v: number | null) => (v === null ? "—" : t("ms", { n: Math.round(v) }));
  const totals = s.days.reduce(
    (a, d) => ({
      requests: a.requests + d.requests,
      chars: a.chars + d.chars,
      units: a.units + d.units,
      llm: a.llm + d.llm_units,
      known: a.known + d.known_units,
      failed: a.failed + d.failed_units,
      first: a.first + d.first_ms,
    }),
    { requests: 0, chars: 0, units: 0, llm: 0, known: 0, failed: 0, first: 0 },
  );
  const card = "rounded-xl border bg-card p-5 shadow-(--shadow-card)";
  const toggle = (field: "mask_pii" | "store" | "browser") => (
    <label className="flex items-start gap-3 text-sm">
      <Switch className="mt-0.5" checked={s.settings[field]} disabled={save.isPending} onCheckedChange={(v) => set({ [field]: v })} />
      <span>
        <span className="font-medium">{t(`settings.${field}.label`)}</span>
        <span className="block text-muted-foreground">{t(`settings.${field}.help`)}</span>
      </span>
    </label>
  );

  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <PageHeader title={t("title")} subtitle={t("subtitle")} />

      <section className={card}>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="max-w-2xl space-y-2 text-sm">
            <h2 className="text-base font-semibold">{t("optIn.title")}</h2>
            <p className="text-muted-foreground">{t("optIn.sent")}</p>
            <p className="text-muted-foreground">{t("optIn.stored")}</p>
          </div>
          <label className="flex items-center gap-2 text-sm font-medium">
            <Switch checked={s.settings.enabled} disabled={save.isPending} onCheckedChange={(v) => set({ enabled: v })} />
            {s.settings.enabled ? t("optIn.on") : t("optIn.off")}
          </label>
        </div>
        {s.settings.enabled && (
          <div className="mt-5 grid gap-4 border-t pt-5 sm:grid-cols-3">
            {toggle("mask_pii")}
            {toggle("store")}
            {toggle("browser")}
          </div>
        )}
      </section>

      {!s.provider.ready && (
        <div role="status" className="flex items-start gap-3 rounded-xl border border-amber-500/40 bg-amber-500/5 p-4 text-sm">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-600" />
          <div>
            <p className="font-medium">{s.provider.name === "mock" ? t("provider.mock") : t("provider.missing")}</p>
            <p className="text-muted-foreground">
              {t.rich("provider.fix", { link: (c) => <Link className="underline" href={`/app/${slug}/settings`}>{c}</Link> })}
            </p>
          </div>
        </div>
      )}

      <section className={card}>
        <h2 className="text-base font-semibold">{t("playground.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("playground.body", { terms: s.glossary_terms })}</p>
        {!s.settings.enabled ? (
          <p className="mt-4 text-sm text-muted-foreground">{t("playground.enableFirst")}</p>
        ) : locales.length === 0 ? (
          <p className="mt-4 text-sm text-muted-foreground">{t("playground.noLocale")}</p>
        ) : (
          <>
            <Textarea
              className="mt-4 min-h-36 font-mono text-[13px]"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t("playground.placeholder")}
              aria-label={t("playground.input")}
              dir="auto"
            />
            <div className="mt-3 flex flex-wrap items-center gap-3 text-sm">
              <label className="flex items-center gap-2">
                {t("playground.to")}
                <select className="h-8 rounded-md border bg-background px-2" value={target} onChange={(e) => setTo(e.target.value)}>
                  {locales.map((l) => (
                    <option key={l} value={l}>{localeName(l)}</option>
                  ))}
                </select>
              </label>
              <label className="flex items-center gap-2">
                {t("playground.speed")}
                <select className="h-8 rounded-md border bg-background px-2" value={speed} onChange={(e) => setSpeed(Number(e.target.value) as (typeof SPEEDS)[number])}>
                  {SPEEDS.map((v) => (
                    <option key={v} value={v}>{v ? t("playground.cps", { n: v }) : t("playground.atOnce")}</option>
                  ))}
                </select>
              </label>
              {running ? (
                <Button variant="outline" onClick={() => stop.current?.abort()}>
                  <Square /> {t("playground.stop")}
                </Button>
              ) : (
                <Button disabled={!text.trim()} onClick={() => void start()}>
                  <Play /> {t("playground.start")}
                </Button>
              )}
              {running && <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden />}
            </div>
            {run && (
              <>
                <div className="mt-4 grid gap-3 md:grid-cols-2">
                  <div>
                    <div className="mb-1 text-xs font-medium text-muted-foreground">{t("playground.source")}</div>
                    <pre dir="auto" className="min-h-32 whitespace-pre-wrap rounded-lg border bg-muted/30 p-3 font-mono text-[13px]">{source}</pre>
                  </div>
                  <div>
                    <div className="mb-1 text-xs font-medium text-muted-foreground">{t("playground.output", { locale: localeName(target) })}</div>
                    <pre dir="auto" lang={target} aria-live="polite" className="min-h-32 whitespace-pre-wrap rounded-lg border bg-muted/30 p-3 font-mono text-[13px]">
                      {run.pieces.map((p, i) =>
                        p.via === "source" ? (
                          <span key={i} title={t(`reason.${reasonKey(p.reason)}`)} className="underline decoration-amber-500 decoration-wavy underline-offset-4">{p.text}</span>
                        ) : p.via === "tm" || p.via === "cache" ? (
                          <span key={i} title={t(`via.${p.via}`)} className="text-emerald-700 dark:text-emerald-400">{p.text}</span>
                        ) : (
                          <span key={i}>{p.text}</span>
                        ),
                      )}
                    </pre>
                  </div>
                </div>
                <dl className="mt-4 grid grid-cols-2 gap-3 text-sm sm:grid-cols-5">
                  <Metric label={t("metric.first")} value={ms(run.first)} />
                  <Metric label={t("metric.lag")} value={ms(median(run.lags))} />
                  <Metric label={t("metric.lagMax")} value={ms(run.lags.length ? Math.max(...run.lags) : null)} />
                  <Metric label={t("metric.llm")} value={count(["llm"])} />
                  <Metric label={t("metric.known")} value={count(["tm", "cache"])} />
                </dl>
                {(reasons.length > 0 || run.fallback) && (
                  <ul className="mt-3 space-y-1 text-sm text-amber-700 dark:text-amber-400">
                    {[...new Set([...reasons, ...(run.fallback ? [run.fallback] : [])])].map((r) => (
                      <li key={r}>{run.pieces.filter((p) => p.reason === r).length} × {t(`reason.${reasonKey(r)}`)}</li>
                    ))}
                  </ul>
                )}
              </>
            )}
          </>
        )}
      </section>

      <section className={card}>
        <h2 className="text-base font-semibold">{t("code.title")}</h2>
        <p className="mt-1 mb-3 text-sm text-muted-foreground">
          {t.rich("code.body", { link: (c) => <Link className="underline" href="/docs/live-stream">{c}</Link> })}
        </p>
        <CodeBlock
          label="app/api/chat/route.ts"
          code={`import { translateStream } from "@glot-i18n/live-stream";

// result.textStream: what your LLM SDK streams (ReadableStream or async iterable)
return new Response(
  translateStream(result.textStream, { project: "${slug}", to: "${target || "es"}", token: process.env.GLOT_TOKEN })
    .pipeThrough(new TextEncoderStream()),
);`}
        />
        {s.settings.browser && (
          <CodeBlock
            className="mt-3"
            label="HTML"
            code={`<script src="https://glotlang.com/widget/glot-live.js" data-project="${slug}" data-key="${s.public_key}" defer></script>
<div data-glot-live="${target || "es"}"><!-- text that streams in here is shown translated --></div>`}
          />
        )}
      </section>

      <section className={card}>
        <h2 className="text-base font-semibold">{t("usage.title")}</h2>
        {totals.requests === 0 ? (
          <EmptyState className="mt-4 border-0 py-8" icon={Radio} title={t("usage.empty.title")} body={t("usage.empty.body")} />
        ) : (
          <>
            <dl className="mt-4 grid grid-cols-2 gap-3 text-sm sm:grid-cols-5">
              <Metric label={t("usage.chars")} value={totals.chars.toLocaleString()} />
              <Metric label={t("usage.units")} value={totals.units.toLocaleString()} />
              <Metric label={t("metric.llm")} value={totals.llm.toLocaleString()} />
              <Metric label={t("metric.known")} value={totals.known.toLocaleString()} />
              <Metric label={t("usage.failed")} value={totals.failed.toLocaleString()} />
            </dl>
            <p className="mt-3 text-sm text-muted-foreground">
              {t("usage.repeats", { repeats: s.units.repeats, distinct: s.units.distinct, stored: s.units.stored })}
            </p>
          </>
        )}
      </section>
    </div>
  );
}

const REASONS = ["no_provider", "provider_error", "provider_timeout", "provider_down", "invalid_output", "glossary", "live_stream_disabled", "live_stream_daily_cap"] as const;
function reasonKey(r?: string): (typeof REASONS)[number] | "other" {
  return (REASONS as readonly string[]).includes(r ?? "") ? (r as (typeof REASONS)[number]) : "other";
}

function Metric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border p-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-lg font-semibold tabular-nums">{value}</dd>
    </div>
  );
}
