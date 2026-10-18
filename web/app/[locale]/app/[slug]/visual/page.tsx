"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowRight, Images, Monitor, Smartphone } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useMemo, useState } from "react";
import { CodeBlock, EmptyState, ErrorState, PageHeader, TermTip, useLocaleName } from "@/components/common";
import { KeySheet, type KeyTarget } from "@/components/key-sheet";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api, type Project, qk, VISUAL_KINDS, type VisualIssue, type VisualKind, type VisualPage, type VisualReport } from "@/lib/api";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";
import { cn } from "@/lib/utils";
import { issueLabel, KIND_STYLE, parseViewport } from "@/lib/visual";

const ALL = "all";
const NONE = "none";
const pageId = (x: { locale: string; route: string; viewport: string }) => `${x.locale}|${x.route}|${x.viewport}`;

export default function VisualPage() {
  const project = useProject(useSlug());
  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  return <VisualQa project={project.data} />;
}

function VisualQa({ project: p }: { project: Project }) {
  const t = useTranslations("visual");
  const format = useFormatter();
  const origin = useOrigin() ?? "";
  const list = useQuery({ queryKey: qk.visualReports(p.slug), queryFn: () => api.visualReports(p.slug) });
  const [current, setCurrent] = useState<string | null>(null);
  const [compare, setCompare] = useState<string | null>(null);
  const reports = list.data ?? [];
  const currentId = current ?? reports[0]?.id;
  const compareId = compare === NONE ? undefined : (compare ?? reports[reports.findIndex((r) => r.id === currentId) + 1]?.id);

  const a = useQuery({
    queryKey: qk.visualReport(p.slug, currentId ?? ""),
    queryFn: () => api.visualReport(p.slug, currentId!),
    enabled: !!currentId,
  });
  const b = useQuery({
    queryKey: qk.visualReport(p.slug, compareId ?? ""),
    queryFn: () => api.visualReport(p.slug, compareId!),
    enabled: !!compareId,
  });

  const [kinds, setKinds] = useState<VisualKind[]>([]);
  const [locale, setLocale] = useState(ALL);
  const [viewport, setViewport] = useState(ALL);
  const [open, setOpen] = useState<VisualPage | null>(null);
  const [sheet, setSheet] = useState<KeyTarget | null>(null);

  const keep = (i: VisualIssue) => !kinds.length || kinds.includes(i.kind);
  const groups = useMemo(() => {
    if (!a.data) return [];
    const byLocale = new Map<string, VisualPage[]>();
    for (const pg of a.data.report.pages) {
      if ((locale !== ALL && pg.locale !== locale) || (viewport !== ALL && pg.viewport !== viewport)) continue;
      byLocale.set(pg.locale, [...(byLocale.get(pg.locale) ?? []), pg]);
    }
    return [...byLocale.entries()];
  }, [a.data, locale, viewport]);
  const issuesOf = (r: VisualReport | undefined, pg: VisualPage) =>
    (r?.report.issues ?? []).filter((i) => pageId(i) === pageId(pg) && keep(i));

  if (list.isPending) return <Skeleton className="mx-auto h-[70vh] max-w-6xl rounded-xl" />;
  if (list.isError) return <ErrorState error={list.error} onRetry={() => list.refetch()} />;

  if (!reports.length) {
    return (
      <div className="mx-auto max-w-3xl">
        <PageHeader title={t("title")} subtitle={t.rich("subtitle", { term: (c) => <TermTip tip={t("budgetTip")}>{c}</TermTip> })} />
        <EmptyState icon={Images} title={t("empty.title")} body={t("empty.body")} />
        <CodeBlock
          className="mt-4"
          label={t("empty.command")}
          code={`GLOT_TOKEN=glot_… npx glot-visual-qa --base-url http://localhost:3000 \\\n  --upload ${origin || "http://localhost:8790"} --project ${p.slug}`}
        />
      </div>
    );
  }

  const meta = (id: string | undefined) => reports.find((r) => r.id === id);
  const reportLabel = (id: string) => {
    const r = meta(id)!;
    return `${format.dateTime(new Date(r.created_at), { dateStyle: "medium", timeStyle: "short" })}${r.label ? ` · ${r.label}` : ""}`;
  };
  const totals = (id: string | undefined) => meta(id)?.summary.by_kind ?? {};

  return (
    <div className="mx-auto max-w-7xl">
      <PageHeader title={t("title")} subtitle={t.rich("subtitle", { term: (c) => <TermTip tip={t("budgetTip")}>{c}</TermTip> })} />

      {/* Reports + kind totals (with delta against the compared report) */}
      <div className="mb-4 flex flex-wrap items-end gap-3">
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground">{t("report")}</span>
          <Select value={currentId} onValueChange={(v) => { setCurrent(v); setCompare(null); }}>
            <SelectTrigger size="sm" className="w-64" aria-label={t("report")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {reports.map((r) => (
                <SelectItem key={r.id} value={r.id}>{reportLabel(r.id)}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <ArrowRight className="mb-2 size-4 text-muted-foreground rtl:rotate-180" aria-hidden />
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground">{t("compareWith")}</span>
          <Select value={compareId ?? NONE} onValueChange={setCompare}>
            <SelectTrigger size="sm" className="w-64" aria-label={t("compareWith")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NONE}>{t("noCompare")}</SelectItem>
              {reports.filter((r) => r.id !== currentId).map((r) => (
                <SelectItem key={r.id} value={r.id}>{reportLabel(r.id)}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="mb-6 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        {VISUAL_KINDS.map((k) => {
          const n = totals(currentId)[k] ?? 0;
          const delta = compareId ? n - (totals(compareId)[k] ?? 0) : 0;
          const on = kinds.includes(k);
          return (
            <button
              key={k}
              type="button"
              aria-pressed={on}
              onClick={() => setKinds(on ? kinds.filter((x) => x !== k) : [...kinds, k])}
              className={cn(
                "rounded-xl border p-3 text-start transition-colors hover:bg-accent/40",
                on && "border-foreground/40 bg-accent/60",
              )}
            >
              <span className={cn("rounded px-1.5 py-0.5 text-[11px] font-medium", KIND_STYLE[k])}>{t(`kinds.${k}`)}</span>
              <div className="mt-2 flex items-baseline gap-2">
                <span className={cn("text-2xl font-semibold tabular-nums", n === 0 && "text-muted-foreground/60")}>{format.number(n)}</span>
                {delta !== 0 && (
                  <span className={cn("font-mono text-xs tabular-nums", delta > 0 ? "text-destructive" : "text-success-foreground")}>
                    {delta > 0 ? "+" : "−"}
                    {format.number(Math.abs(delta))}
                  </span>
                )}
              </div>
            </button>
          );
        })}
      </div>

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <Select value={locale} onValueChange={setLocale}>
          <SelectTrigger size="sm" className="w-40" aria-label={t("locale")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>{t("allLocales")}</SelectItem>
            {(meta(currentId)?.summary.locales ?? []).map((l) => (
              <SelectItem key={l} value={l}>{l}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={viewport} onValueChange={setViewport}>
          <SelectTrigger size="sm" className="w-40" aria-label={t("viewport")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>{t("allViewports")}</SelectItem>
            {(meta(currentId)?.summary.viewports ?? []).map((v) => (
              <SelectItem key={v} value={v}>{v}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        {kinds.length > 0 && (
          <button type="button" className="text-xs text-muted-foreground underline-offset-2 hover:underline" onClick={() => setKinds([])}>
            {t("clearKinds")}
          </button>
        )}
      </div>

      {a.isPending ? (
        <Skeleton className="h-96 rounded-xl" />
      ) : a.isError ? (
        <ErrorState error={a.error} onRetry={() => a.refetch()} />
      ) : (
        <div className="space-y-8">
          {groups.map(([loc, pages]) => (
            <LocaleGroup key={loc} locale={loc}>
              {pages
                .filter((pg) => !kinds.length || issuesOf(a.data, pg).length > 0)
                .map((pg) => (
                  <Thumb
                    key={pageId(pg)}
                    slug={p.slug}
                    report={a.data!}
                    page={pg}
                    issues={issuesOf(a.data, pg)}
                    before={b.data ? issuesOf(b.data, pg).length : undefined}
                    onOpen={() => setOpen(pg)}
                  />
                ))}
            </LocaleGroup>
          ))}
        </div>
      )}

      <Dialog open={!!open} onOpenChange={(o) => !o && setOpen(null)}>
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-6xl">
          {open && a.data && (
            <>
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <span className="rounded border px-1.5 font-mono text-xs">{open.locale}</span>
                  <span className="font-mono text-sm">{open.route}</span>
                  <span className="text-xs font-normal text-muted-foreground">{open.viewport}</span>
                </DialogTitle>
                <DialogDescription dir="ltr" className="truncate font-mono text-xs">{open.url}</DialogDescription>
              </DialogHeader>
              <div className={cn("grid gap-4", b.data && "md:grid-cols-2")}>
                {b.data && (
                  <Shot
                    title={t("before")}
                    slug={p.slug}
                    report={b.data}
                    page={b.data.report.pages.find((x) => pageId(x) === pageId(open))}
                    issues={issuesOf(b.data, open)}
                    onIssue={(i) => setSheet({ locale: i.locale, key: i.key, text: i.text, issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}` })}
                  />
                )}
                <Shot
                  title={b.data ? t("after") : undefined}
                  slug={p.slug}
                  report={a.data}
                  page={open}
                  issues={issuesOf(a.data, open)}
                  onIssue={(i) => setSheet({ locale: i.locale, key: i.key, text: i.text, issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}` })}
                />
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>
      <KeySheet project={p} target={sheet} onClose={() => setSheet(null)} />
    </div>
  );
}

function LocaleGroup({ locale, children }: { locale: string; children: React.ReactNode }) {
  const localeName = useLocaleName();
  return (
    <section>
      <h2 className="mb-3 flex items-center gap-2 text-sm font-medium">
        <span className="rounded border px-1.5 font-mono text-xs">{locale}</span>
        <span className="text-muted-foreground capitalize">{localeName(locale)}</span>
      </h2>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">{children}</div>
    </section>
  );
}

function Thumb({
  slug,
  report,
  page,
  issues,
  before,
  onOpen,
}: {
  slug: string;
  report: VisualReport;
  page: VisualPage;
  issues: VisualIssue[];
  before?: number;
  onOpen: () => void;
}) {
  const t = useTranslations("visual");
  const { w } = parseViewport(page.viewport);
  const Icon = w < 900 ? Smartphone : Monitor;
  const delta = before === undefined ? 0 : issues.length - before;
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group overflow-hidden rounded-xl border text-start transition-shadow hover:shadow-md focus-visible:ring-2 focus-visible:ring-ring"
    >
      <div className="relative h-44 overflow-hidden border-b bg-muted/40">
        {/* eslint-disable-next-line @next/next/no-img-element -- authenticated API image, not optimisable */}
        <img
          src={api.visualFileUrl(slug, report.id, page.screenshot)}
          alt={t("screenshotAlt", { locale: page.locale, route: page.route })}
          loading="lazy"
          className={cn("w-full object-cover object-top transition-transform duration-300 group-hover:scale-[1.02]", w < 900 && "mx-auto w-1/2")}
        />
        {issues.length > 0 && (
          <span className="absolute end-2 top-2 rounded-full bg-destructive px-2 py-0.5 font-mono text-[11px] font-medium text-white tabular-nums">
            {issues.length}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 px-3 py-2 text-sm">
        <Icon className="size-3.5 text-muted-foreground" aria-hidden />
        <span className="truncate font-mono text-xs">{page.route}</span>
        <span className="text-xs text-muted-foreground">{page.viewport}</span>
        <span className="ms-auto text-xs">
          {issues.length === 0 ? (
            <span className="text-success-foreground">{t("clean")}</span>
          ) : (
            <span className="text-muted-foreground">{t("issues", { count: issues.length })}</span>
          )}
          {delta !== 0 && (
            <span className={cn("ms-1 font-mono", delta > 0 ? "text-destructive" : "text-success-foreground")}>
              {delta > 0 ? `+${delta}` : `−${-delta}`}
            </span>
          )}
        </span>
      </div>
    </button>
  );
}

function Shot({
  title,
  slug,
  report,
  page,
  issues,
  onIssue,
}: {
  title?: string;
  slug: string;
  report: VisualReport;
  page?: VisualPage;
  issues: VisualIssue[];
  onIssue: (i: VisualIssue) => void;
}) {
  const t = useTranslations("visual");
  const format = useFormatter();
  const [cssHeight, setCssHeight] = useState<number | null>(null);
  const { w } = parseViewport(page?.viewport ?? "");
  return (
    <div className={cn("min-w-0 gap-3", title ? "space-y-3" : "grid lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start")}>
      {title && (
        <p className="text-xs font-medium text-muted-foreground">
          {title} · {format.dateTime(new Date(report.created_at), { dateStyle: "medium", timeStyle: "short" })}
        </p>
      )}
      {!page ? (
        <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">{t("notInReport")}</p>
      ) : (
        <div className="max-h-[70dvh] overflow-y-auto rounded-lg border" dir="ltr">
          <div className="relative">
          {/* eslint-disable-next-line @next/next/no-img-element -- authenticated API image */}
          <img
            src={api.visualFileUrl(slug, report.id, page.screenshot)}
            alt={t("screenshotAlt", { locale: page.locale, route: page.route })}
            className="w-full"
            onLoad={(e) => setCssHeight((e.currentTarget.naturalHeight * w) / e.currentTarget.naturalWidth)}
          />
          {/* Rects are CSS px at the viewport width; the screenshot may be device-scaled, so position in % of the page. */}
          {cssHeight &&
            issues
              .filter((i) => i.rect)
              .map((i, n) => (
                <Tooltip key={n}>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      onClick={() => onIssue(i)}
                      aria-label={t("issueAt", { kind: i.kind, key: issueLabel(i) })}
                      className="absolute rounded-sm bg-destructive/10 outline-2 outline-destructive hover:bg-destructive/25"
                      style={{
                        left: `${(i.rect!.x / w) * 100}%`,
                        top: `${(i.rect!.y / cssHeight) * 100}%`,
                        width: `${(i.rect!.width / w) * 100}%`,
                        height: `${(i.rect!.height / cssHeight) * 100}%`,
                      }}
                    />
                  </TooltipTrigger>
                  <TooltipContent className="max-w-xs">
                    <p className="font-medium">{t(`kinds.${i.kind}`)}</p>
                    <p className="font-mono opacity-80">{issueLabel(i)}</p>
                  </TooltipContent>
                </Tooltip>
              ))}
          </div>
        </div>
      )}
      {issues.length > 0 && (
        <ul className={cn("divide-y rounded-lg border", !title && "lg:max-h-[70dvh] lg:overflow-y-auto")}>
          {issues.map((i, n) => (
            <li key={n}>
              <button type="button" onClick={() => onIssue(i)} className="flex w-full items-start gap-3 px-3 py-2 text-start hover:bg-accent/40">
                <span className={cn("mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium", KIND_STYLE[i.kind])}>{t(`kinds.${i.kind}`)}</span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-xs">{issueLabel(i)}</span>
                  <span dir="auto" className="block truncate text-xs text-muted-foreground">{i.text}</span>
                  {i.suggestion && <span dir="auto" className="mt-0.5 block text-xs text-success-foreground">→ {i.suggestion}</span>}
                </span>
                {i.crop && (
                  // eslint-disable-next-line @next/next/no-img-element -- authenticated API image
                  <img src={api.visualFileUrl(slug, report.id, i.crop)} alt="" loading="lazy" className="h-10 max-w-40 shrink-0 rounded border object-contain object-left" />
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
