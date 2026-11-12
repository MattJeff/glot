"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Gauge, Loader2, Play, SearchCheck, Zap } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, EmptyState, ErrorState } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import type { Project } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { edgeApi, edgeKeys, type EdgeSlug, type SeoLocaleResult, type SeoReport, type SeoReportPage } from "./api";

/** Issue codes of seo_report.rs, worst first (unknown codes are shown as is). */
const ISSUES = [
  "fetch_failed", "http_error", "noindex", "robots_txt_blocked", "redirected", "canonical_missing", "canonical_multiple",
  "canonical_to_source", "canonical_not_self", "html_lang", "html_dir", "content_language", "hreflang_self_missing",
  "hreflang_x_default_missing", "hreflang_not_reciprocal", "advertised_but_not_ready", "title_missing", "title_untranslated",
  "title_duplicate", "description_missing", "description_untranslated", "og_title_untranslated", "jsonld_language", "untranslated_segments",
] as const;
type IssueCode = (typeof ISSUES)[number];
const known = (c: string): c is IssueCode => (ISSUES as readonly string[]).includes(c);

function useIssueLabel() {
  const t = useTranslations("edge.seo.issues");
  return (c: string) => (known(c) ? t(c) : c);
}

const pill = (ok: boolean | null, label: string) => (
  <Badge
    variant="secondary"
    className={cn(ok === true && "bg-success/12 text-success-foreground", ok === false && "bg-destructive/10 text-destructive", ok === null && "text-muted-foreground")}
  >
    {label}
  </Badge>
);

/** The report as a spreadsheet: one row per page and language (opens in Excel / Sheets). */
function downloadCsv(r: SeoReport) {
  const cell = (v: unknown) => `"${String(v ?? "").replace(/"/g, '""')}"`;
  const head = ["path", "locale", "url", "status", "indexable", "canonical", "title", "description", "untranslated", "issues"];
  const rows = r.pages.flatMap((p) =>
    p.locales.map((l) => [p.path, l.locale, l.url, l.status, l.technically_indexable, l.canonical, l.title, l.description, l.untranslated.count, l.issues.join(" ")]),
  );
  const csv = [head, ...rows].map((x) => x.map(cell).join(",")).join("\r\n");
  const a = Object.assign(document.createElement("a"), {
    href: URL.createObjectURL(new Blob(["\ufeff" + csv], { type: "text/csv;charset=utf-8" })),
    download: `seo-report-${r.generated_at.slice(0, 10)}.csv`,
  });
  a.click();
  URL.revokeObjectURL(a.href);
}

/**
 * The outcome first: per language, the pages a search engine is told about (complete pages only), and the
 * pre-warm that fills them — one click translates what is still missing, within the project's caps.
 */
function SeoOutcome({ slug }: { slug: string }) {
  const t = useTranslations("edge.seo.outcome");
  const format = useFormatter();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const coverage = useQuery({ queryKey: edgeKeys.coverage(slug), queryFn: () => edgeApi.coverage(slug) });
  const prewarm = useQuery({
    queryKey: edgeKeys.prewarm(slug),
    queryFn: () => edgeApi.prewarm(slug),
    refetchInterval: (q) => (q.state.data?.status === "running" ? 2000 : false),
  });
  const run = useMutation({
    mutationFn: () => edgeApi.runPrewarm(slug),
    onSuccess: (r) => qc.setQueryData(edgeKeys.prewarm(slug), r),
    onError: (e) => toast.error(message(e)),
  });
  const p = prewarm.data;
  const running = p?.status === "running" || run.isPending;
  return (
    <section className="space-y-3 rounded-xl border p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="font-medium">{t("title")}</h2>
        <Button className="ms-auto" size="sm" onClick={() => run.mutate()} disabled={running}>
          {running ? <Loader2 className="animate-spin" /> : <Zap />}
          {running ? t("running", { done: p?.pages_done ?? 0, total: p?.pages_total ?? 0 }) : t("run")}
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">{t("body")}</p>
      {coverage.data && (
        <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {coverage.data.locales.map((l) => (
            <div key={l.locale} className="rounded-lg border p-3">
              <dt className="flex items-center gap-2 text-xs text-muted-foreground">
                <span className="font-mono">{l.locale}</span>
                {pill(l.ready, l.ready ? t("advertised") : t("notAdvertised"))}
              </dt>
              <dd className="mt-1 text-xl font-semibold tabular-nums">
                {format.number(l.pages_indexable)} <span className="text-sm font-normal text-muted-foreground">/ {format.number(l.pages_seen)}</span>
              </dd>
              <dd className="text-xs text-muted-foreground">{t("pages")}</dd>
            </div>
          ))}
        </dl>
      )}
      {p && p.status !== "never" && (
        <p className={cn("text-xs text-muted-foreground", p.status === "failed" && "text-destructive")}>
          {p.status === "failed"
            ? t("failed", { error: p.error ?? "" })
            : t("last", {
                pages: p.pages_done ?? 0,
                skipped: p.skipped ?? 0,
                segments: p.segments ?? 0,
                date: format.dateTime(new Date(p.finished_at ?? p.started_at ?? 0), { dateStyle: "medium", timeStyle: "short" }),
              })}
        </p>
      )}
    </section>
  );
}

/** Translated page addresses (opt-in in the settings): each one can be replaced; the old address redirects. */
function SeoSlugs({ slug }: { slug: string }) {
  const t = useTranslations("edge.seo.slugs");
  const qc = useQueryClient();
  const message = useErrorMessage();
  const slugs = useQuery({ queryKey: edgeKeys.slugs(slug), queryFn: () => edgeApi.slugs(slug) });
  const put = useMutation({
    mutationFn: (b: { locale: string; source_path: string; slug: string }) => edgeApi.putSlug(slug, b),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: edgeKeys.slugs(slug) });
      toast.success(t("saved"));
    },
    onError: (e) => toast.error(message(e)),
  });
  if (!slugs.data?.enabled) return null;
  const save = (row: EdgeSlug, value: string) => {
    const v = value.trim();
    if (v && v !== row.slug) put.mutate({ locale: row.locale, source_path: row.source_path, slug: v });
  };
  return (
    <details className="rounded-xl border p-4">
      <summary className="cursor-pointer font-medium">{t("title", { n: slugs.data.slugs.length })}</summary>
      <p className="mt-2 text-sm text-muted-foreground">{t("body")}</p>
      {slugs.data.slugs.length > 0 && (
        <table className="mt-3 w-full text-sm">
          <thead className="text-xs text-muted-foreground">
            <tr>
              <th className="py-1 text-start font-normal">{t("source")}</th>
              <th className="py-1 text-start font-normal">{t("locale")}</th>
              <th className="py-1 text-start font-normal">{t("address")}</th>
            </tr>
          </thead>
          <tbody>
            {slugs.data.slugs.map((row) => (
              <tr key={row.locale + row.source_path} className="border-t">
                <td dir="ltr" translate="no" className="py-1.5 font-mono text-xs">
                  {row.source_path}
                </td>
                <td className="py-1.5 font-mono text-xs">{row.locale}</td>
                <td className="py-1.5">
                  <Input
                    key={row.slug}
                    defaultValue={row.slug}
                    aria-label={t("addressOf", { path: row.source_path, locale: row.locale })}
                    dir="ltr"
                    translate="no"
                    className="h-8 font-mono text-xs"
                    onBlur={(e) => save(row, e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </details>
  );
}

/**
 * International SEO report = "View as Google": each advertised locale of up to 20 pages fetched raw (no JavaScript),
 * "technically indexable" vs "actually indexed" (unknown without Search Console, never guessed).
 */
export function SeoReportView({ project }: { project: Project }) {
  const t = useTranslations("edge.seo");
  const format = useFormatter();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const label = useIssueLabel();
  const report = useQuery({ queryKey: edgeKeys.seo(project.slug), queryFn: () => edgeApi.seoReport(project.slug) });
  const run = useMutation({
    mutationFn: () => edgeApi.runSeoReport(project.slug),
    onSuccess: (r) => qc.setQueryData(edgeKeys.seo(project.slug), r),
    onError: (e) => toast.error(message(e)),
  });
  const ping = useMutation({
    mutationFn: () => edgeApi.indexNow(project.slug),
    onSuccess: (r) => toast.success(t("indexNowDone", { n: r.hosts.reduce((n, h) => n + h.urls, 0) })),
    onError: (e) => toast.error(message(e)),
  });
  const button = (
    <Button onClick={() => run.mutate()} disabled={run.isPending}>
      {run.isPending ? <Loader2 className="animate-spin" /> : <Play />}
      {run.isPending ? t("running") : t("run")}
    </Button>
  );

  const outcome = (
    <>
      <SeoOutcome slug={project.slug} />
      <SeoSlugs slug={project.slug} />
    </>
  );
  if (report.isPending) return <Skeleton className="h-64 rounded-xl" />;
  if (report.isError) return <ErrorState error={report.error} onRetry={() => report.refetch()} />;
  const r = report.data;
  if (!r)
    return (
      <div className="space-y-6">
        {outcome}
        <EmptyState icon={SearchCheck} title={t("empty.title")} body={t("empty.body")} action={button} />
      </div>
    );
  const rows = r.pages.flatMap((p) => p.locales);
  const indexable = rows.filter((l) => l.technically_indexable).length;
  const totals = Object.entries(r.issues).sort((a, b) => b[1] - a[1]);

  return (
    <div className="space-y-6">
      {outcome}
      <div className="flex flex-wrap items-center gap-3">
        <p className="text-sm text-muted-foreground">
          {t("summary", { indexable, total: rows.length })} · {format.dateTime(new Date(r.generated_at), { dateStyle: "medium", timeStyle: "short" })}
        </p>
        <div className="ms-auto flex gap-2">
          <Button variant="outline" onClick={() => downloadCsv(r)}>
            <Download />
            {t("export")}
          </Button>
          {button}
        </div>
      </div>
      <p className="rounded-lg border bg-muted/40 p-3 text-xs text-muted-foreground">{t("indexedNote")}</p>
      {totals.length > 0 && (
        <ul className="flex flex-wrap gap-1.5" aria-label={t("issuesTitle")}>
          {totals.map(([code, n]) => (
            <li key={code}>
              <Badge variant="outline" className="gap-1.5">
                {label(code)} <span className="tabular-nums text-muted-foreground">{n}</span>
              </Badge>
            </li>
          ))}
        </ul>
      )}
      <ul className="space-y-4">
        {r.pages.map((p) => (
          <SeoPageCard key={p.path} slug={project.slug} page={p} />
        ))}
      </ul>
      {r.sitemaps.length > 0 && (
        <section className="space-y-2">
          <CodeBlock label={t("sitemaps")} code={r.sitemaps.join("\n")} />
          <div className="flex flex-wrap items-center gap-3">
            <Button variant="outline" size="sm" onClick={() => ping.mutate()} disabled={ping.isPending}>
              {ping.isPending && <Loader2 className="animate-spin" />}
              {t("indexNow")}
            </Button>
            <p className="text-xs text-muted-foreground">{t("indexNowHint")}</p>
          </div>
        </section>
      )}
    </div>
  );
}

function SeoPageCard({ slug, page: p }: { slug: string; page: SeoReportPage }) {
  const t = useTranslations("edge.seo");
  const r = p.render;
  return (
    <li className="rounded-xl border">
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-3">
        <span dir="ltr" translate="no" className="font-mono text-sm font-medium">
          {p.path}
        </span>
        {p.critical && <Badge variant="outline">{t("critical")}</Badge>}
        <span className="ms-auto text-xs text-muted-foreground">
          {r.status === "same"
            ? t("renderDiffSame")
            : r.status === "js_only_content"
              ? t("renderDiffCount", { n: r.only_after_js ?? 0 })
              : t("renderDiffNone")}
        </span>
      </div>
      <ul className="divide-y">
        {p.locales.map((l) => (
          <SeoLocaleRow key={l.locale} slug={slug} l={l} />
        ))}
      </ul>
    </li>
  );
}

function SeoLocaleRow({ slug, l }: { slug: string; l: SeoLocaleResult }) {
  const t = useTranslations("edge.seo");
  const label = useIssueLabel();
  const hreflang = Object.entries(l.hreflang);
  const mono = (v: string | null) => (v ? <span dir="ltr" translate="no" className="font-mono text-xs break-all">{v}</span> : "—");
  return (
    <li className="px-4 py-3">
      <details>
        <summary className="flex cursor-pointer flex-wrap items-center gap-2">
          <span className="w-10 font-mono text-xs text-muted-foreground">{l.locale}</span>
          <span dir="ltr" translate="no" className="min-w-0 flex-1 truncate font-mono text-xs">
            {l.url}
          </span>
          {l.status !== 200 && pill(false, l.error ?? String(l.status ?? "—"))}
          {pill(l.technically_indexable, l.technically_indexable ? t("indexable") : t("notIndexable"))}
          {pill(l.indexed, l.indexed === null ? t("indexedUnknown") : l.indexed ? t("indexed") : t("notIndexed"))}
          {l.not_reciprocal.length > 0 && pill(false, t("notReciprocal"))}
          {l.untranslated.count > 0 && pill(false, t("untranslated", { n: l.untranslated.count }))}
        </summary>
        <dl className="mt-3 grid gap-x-6 gap-y-1.5 text-sm sm:grid-cols-[10rem_1fr]">
          <dt className="text-muted-foreground">{t("fields.lang")}</dt>
          <dd>{mono([l.lang, l.dir].filter(Boolean).join(" / ") || null)}</dd>
          <dt className="text-muted-foreground">{t("fields.contentLanguage")}</dt>
          <dd>{mono(l.content_language)}</dd>
          <dt className="text-muted-foreground">{t("fields.canonical")}</dt>
          <dd>{mono(l.canonical)}</dd>
          <dt className="text-muted-foreground">{t("fields.title")}</dt>
          <dd>{l.title ?? "—"}</dd>
          <dt className="text-muted-foreground">{t("fields.description")}</dt>
          <dd>{l.description ?? "—"}</dd>
          <dt className="text-muted-foreground">{t("fields.hreflang")}</dt>
          <dd>
            {hreflang.length === 0 ? (
              "—"
            ) : (
              <ul dir="ltr" className="space-y-0.5 font-mono text-xs">
                {hreflang.map(([lang, href]) => (
                  <li key={lang} className={cn(l.not_reciprocal.includes(lang) && "text-warning-foreground")}>
                    {lang} → {href}
                  </li>
                ))}
              </ul>
            )}
          </dd>
        </dl>
        {l.issues.length > 0 && (
          <ul className="mt-3 flex flex-wrap gap-1.5">
            {l.issues.map((c) => (
              <li key={c}>
                <Badge variant="secondary" className="bg-warning/15 text-warning-foreground">
                  {label(c)}
                </Badge>
              </li>
            ))}
          </ul>
        )}
        {l.untranslated.sample.length > 0 && (
          <ul className="mt-3 flex flex-wrap gap-1.5">
            {l.untranslated.sample.map((s) => (
              <li key={s.hash + s.role}>
                <Link
                  href={`/app/${slug}/keys?view=segments&locale=${l.locale}&q=${encodeURIComponent(s.text)}`}
                  className="inline-block max-w-80 truncate rounded-md border px-2 py-0.5 text-xs hover:bg-accent"
                >
                  {s.text}
                </Link>
              </li>
            ))}
          </ul>
        )}
      </details>
    </li>
  );
}

/** Served pages, cache hit ratio, fail-open count and glot's own latency (Server-Timing). */
export function Performance({ slug }: { slug: string }) {
  const t = useTranslations("edge.perf");
  const format = useFormatter();
  const [days, setDays] = useState(7);
  const m = useQuery({ queryKey: [...edgeKeys.metrics(slug), days], queryFn: () => edgeApi.metrics(slug, days) });
  if (m.isPending) return <Skeleton className="h-64 rounded-xl" />;
  if (m.isError) return <ErrorState error={m.error} onRetry={() => m.refetch()} />;
  const d = m.data;
  if (d.requests === 0) return <EmptyState icon={Gauge} title={t("empty.title")} body={t("empty.body")} />;
  const ms = (v: number | null) => (v === null ? "—" : `${format.number(v, { maximumFractionDigits: 0 })} ms`);
  const max = Math.max(1, ...d.by_day.map((x) => x.requests));
  const tiles = [
    [t("requests"), format.number(d.requests)],
    [t("hitRatio"), format.number(d.cache_hit_ratio, { style: "percent", maximumFractionDigits: 1 })],
    [t("failOpen"), format.number(d.fail_open)],
    [t("glotP50"), ms(d.glot_p50_ms)],
    [t("glotP95"), ms(d.glot_p95_ms)],
    [t("originP50"), ms(d.origin_p50_ms)],
  ];
  return (
    <div className="space-y-6">
      <div className="flex gap-1" role="group" aria-label={t("period")}>
        {[1, 7, 30].map((n) => (
          <Button key={n} size="sm" variant={n === days ? "secondary" : "ghost"} aria-pressed={n === days} onClick={() => setDays(n)}>
            {t("days", { n })}
          </Button>
        ))}
      </div>
      <dl className="grid gap-3 sm:grid-cols-3">
        {tiles.map(([label, value]) => (
          <div key={label} className="rounded-xl border p-4">
            <dt className="text-xs text-muted-foreground">{label}</dt>
            <dd className="mt-1 text-xl font-semibold tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
      <p className="text-xs text-muted-foreground">{t("note")}</p>
      <table className="w-full text-sm">
        <caption className="sr-only">{t("byDay")}</caption>
        <thead className="text-xs text-muted-foreground">
          <tr>
            <th className="py-1 text-start font-normal">{t("date")}</th>
            <th className="py-1 text-start font-normal">{t("requests")}</th>
            <th className="py-1 text-end font-normal">{t("hitRatio")}</th>
            <th className="py-1 text-end font-normal">{t("failOpen")}</th>
            <th className="py-1 text-end font-normal">{t("glotP95")}</th>
          </tr>
        </thead>
        <tbody className="tabular-nums">
          {d.by_day.map((x) => (
            <tr key={x.date} className="border-t">
              <td className="py-1.5">{format.dateTime(new Date(x.date), { dateStyle: "medium" })}</td>
              <td className="py-1.5">
                <div className="flex items-center gap-2">
                  <div aria-hidden className="h-1.5 rounded-full bg-brand/60" style={{ width: `${(x.requests / max) * 8}rem` }} />
                  {format.number(x.requests)}
                </div>
              </td>
              <td className="py-1.5 text-end">{x.requests ? format.number(x.hits / x.requests, { style: "percent", maximumFractionDigits: 0 }) : "—"}</td>
              <td className={cn("py-1.5 text-end", x.fail_open > 0 && "text-warning-foreground")}>{format.number(x.fail_open)}</td>
              <td className="py-1.5 text-end">{ms(x.glot_p95_ms)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
