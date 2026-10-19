"use client";

import { useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  CircleCheck,
  Images,
  KeyRound,
  Monitor,
  Play,
  ScanSearch,
  Smartphone,
  TriangleAlert,
} from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useMemo, useState } from "react";
import {
  CodeBlock,
  EmptyState,
  ErrorState,
  PageHeader,
  TermTip,
  useLocaleName,
} from "@/components/common";
import { KeySheet, type KeyTarget } from "@/components/key-sheet";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Link } from "@/i18n/navigation";
import {
  api,
  type Project,
  qk,
  VISUAL_KINDS,
  type VisualIssue,
  type VisualKind,
  type VisualPage,
  type VisualReport,
} from "@/lib/api";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";
import { cn } from "@/lib/utils";
import {
  isExpected,
  issueLabel,
  issueSig,
  KIND_STYLE,
  parseViewport,
  previewTarget,
  runCommand,
} from "@/lib/visual";

const ALL = "all";
const NONE = "none";
const pageId = (x: { locale: string; route: string; viewport: string }) =>
  `${x.locale}|${x.route}|${x.viewport}`;
const rank = (k: VisualKind) => VISUAL_KINDS.indexOf(k);

export default function VisualPage() {
  const project = useProject(useSlug());
  if (project.isError)
    return (
      <ErrorState error={project.error} onRetry={() => project.refetch()} />
    );
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  return <VisualQa project={project.data} />;
}

/** How to produce a report: it is a CLI run on the user's machine or CI, prefilled with this project's settings. */
function RunPanel({
  project: p,
  last,
}: {
  project: Project;
  last?: VisualReport;
}) {
  const t = useTranslations("visual.run");
  const origin = useOrigin() ?? "http://localhost:8790";
  const preview = useQuery({
    queryKey: qk.preview(p.slug),
    queryFn: () => api.preview(p.slug),
  });
  const target = previewTarget(preview.data, last);
  return (
    <section
      aria-labelledby="vqa-run"
      className="mb-6 space-y-3 rounded-xl border p-4"
    >
      <h2 id="vqa-run" className="flex items-center gap-2 font-medium">
        <Play className="size-4" aria-hidden />
        {t("title")}
      </h2>
      <p className="text-sm text-muted-foreground">{t("body")}</p>
      <ol className="list-decimal space-y-1 ps-5 text-sm">
        <li>
          {t.rich("token", {
            link: (c) => (
              <Link
                href="/app/settings/tokens"
                className="font-medium underline underline-offset-4"
              >
                {c}
              </Link>
            ),
          })}
        </li>
        <li>{t("paste")}</li>
      </ol>
      {preview.isPending ? (
        <Skeleton className="h-28 rounded-lg" />
      ) : (
        <CodeBlock code={runCommand(p, origin, target)} />
      )}
      {!preview.isPending && !target.base && (
        <p className="text-xs text-muted-foreground">
          {t.rich("noSite", {
            link: (c) => (
              <Link
                href={`/app/${p.slug}/settings#s-preview-base`}
                className="font-medium text-foreground underline underline-offset-4"
              >
                {c}
              </Link>
            ),
          })}
        </p>
      )}
      <p className="text-xs text-muted-foreground">{t("browser")}</p>
    </section>
  );
}

function VisualQa({ project: p }: { project: Project }) {
  const t = useTranslations("visual");
  const format = useFormatter();
  const list = useQuery({
    queryKey: qk.visualReports(p.slug),
    queryFn: () => api.visualReports(p.slug),
  });
  const [current, setCurrent] = useState<string | null>(null);
  const [compare, setCompare] = useState<string | null>(null);
  const [showRun, setShowRun] = useState(false);
  const reports = list.data ?? [];
  const currentId = current ?? reports[0]?.id;
  const compareId =
    compare === NONE
      ? undefined
      : (compare ??
        reports[reports.findIndex((r) => r.id === currentId) + 1]?.id);

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
  const [route, setRoute] = useState(ALL);
  const [showExpected, setShowExpected] = useState(false);
  const [onlyNew, setOnlyNew] = useState(false);
  const [open, setOpen] = useState<{
    page: VisualPage;
    focus?: VisualIssue;
  } | null>(null);
  const [sheet, setSheet] = useState<KeyTarget | null>(null);

  const src = p.source_locale;
  const where = (x: { locale: string; route: string; viewport: string }) =>
    (locale === ALL || x.locale === locale) &&
    (viewport === ALL || x.viewport === viewport) &&
    (route === ALL || x.route === route);
  // Location filters minus expected noise. Picking the "clamp" card shows clamps even with the toggle off.
  const counted = (r: VisualReport | undefined) =>
    (r?.report.issues ?? []).filter(
      (i) =>
        where(i) && (showExpected || i.kind === "clamp" || !isExpected(i, src)),
    );
  const located = (r: VisualReport | undefined) =>
    counted(r).filter(
      (i) => showExpected || i.kind !== "clamp" || kinds.includes("clamp"),
    );
  const before = useMemo(
    () => new Set((b.data?.report.issues ?? []).map(issueSig)),
    [b.data],
  );
  const isNew = (i: VisualIssue) => !!b.data && !before.has(issueSig(i));
  const keep = (i: VisualIssue) =>
    (!kinds.length || kinds.includes(i.kind)) && (!onlyNew || isNew(i));

  const all = located(a.data);
  const visible = all.filter(keep);
  const hidden = (a.data?.report.issues ?? []).filter(
    (i) => where(i) && isExpected(i, src),
  ).length;
  const after = new Set(all.map(issueSig));
  const fixed = b.data
    ? located(b.data).filter((i) => !after.has(issueSig(i))).length
    : 0;
  const added = all.filter(isNew).length;

  // Route × locale groups, worst first (most severe kind, then most issues).
  const by = new Map<string, VisualIssue[]>();
  for (const i of visible)
    by.set(`${i.route}|${i.locale}`, [
      ...(by.get(`${i.route}|${i.locale}`) ?? []),
      i,
    ]);
  for (const xs of by.values()) xs.sort((x, y) => rank(x.kind) - rank(y.kind));
  const groups = [...by.values()].sort(
    (x, y) => rank(x[0].kind) - rank(y[0].kind) || y.length - x.length,
  );
  const pages = (a.data?.report.pages ?? []).filter(where);
  const issuesOf = (r: VisualReport | undefined, pg: VisualPage) =>
    located(r).filter(
      (i) =>
        pageId(i) === pageId(pg) && (!kinds.length || kinds.includes(i.kind)),
    );
  const pageOf = (i: VisualIssue) =>
    a.data?.report.pages.find((pg) => pageId(pg) === pageId(i));
  const toKey = (i: VisualIssue) =>
    setSheet({
      locale: i.locale,
      key: i.key,
      text: i.text,
      issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}`,
    });
  const resetFilters = () => {
    setKinds([]);
    setLocale(ALL);
    setViewport(ALL);
    setRoute(ALL);
    setOnlyNew(false);
  };

  const header = (
    <PageHeader
      title={t("title")}
      subtitle={t.rich("subtitle", {
        term: (c) => <TermTip tip={t("budgetTip")}>{c}</TermTip>,
      })}
      actions={
        reports.length > 0 && (
          <Button
            variant="outline"
            size="sm"
            aria-expanded={showRun}
            aria-controls="vqa-run-panel"
            onClick={() => setShowRun(!showRun)}
          >
            <Play aria-hidden />
            {t("run.button")}
          </Button>
        )
      }
    />
  );

  if (list.isPending)
    return <Skeleton className="mx-auto h-[70vh] max-w-6xl rounded-xl" />;
  if (list.isError)
    return <ErrorState error={list.error} onRetry={() => list.refetch()} />;

  if (!reports.length) {
    return (
      <div className="mx-auto max-w-3xl">
        {header}
        <EmptyState
          icon={Images}
          title={t("empty.title")}
          body={t("empty.body")}
          className="mb-6"
        />
        <RunPanel project={p} />
      </div>
    );
  }

  const meta = (id: string | undefined) => reports.find((r) => r.id === id);
  const reportLabel = (id: string) => {
    const r = meta(id)!;
    return `${format.dateTime(new Date(r.created_at), { dateStyle: "medium", timeStyle: "short" })}${r.label ? ` · ${r.label}` : ""}`;
  };
  const m = meta(currentId)?.summary;
  const opts = a.data?.report.options;
  const stale = opts && opts.sourceLocale !== src;

  return (
    <div className="mx-auto max-w-7xl">
      {header}
      {showRun && (
        <div id="vqa-run-panel">
          <RunPanel project={p} last={a.data} />
        </div>
      )}

      <div className="mb-4 flex flex-wrap items-end gap-3">
        <div className="min-w-0 space-y-1">
          <span className="text-xs text-muted-foreground">{t("report")}</span>
          <Select
            value={currentId}
            onValueChange={(v) => {
              setCurrent(v);
              setCompare(null);
            }}
          >
            <SelectTrigger
              size="sm"
              className="w-64 max-w-full"
              aria-label={t("report")}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {reports.map((r) => (
                <SelectItem key={r.id} value={r.id}>
                  {reportLabel(r.id)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <ArrowRight
          className="mb-2 hidden size-4 text-muted-foreground sm:block rtl:rotate-180"
          aria-hidden
        />
        <div className="min-w-0 space-y-1">
          <span className="text-xs text-muted-foreground">
            {t("compareWith")}
          </span>
          <Select
            value={compareId ?? NONE}
            onValueChange={(v) => {
              setCompare(v);
              if (v === NONE) setOnlyNew(false);
            }}
          >
            <SelectTrigger
              size="sm"
              className="w-64 max-w-full"
              aria-label={t("compareWith")}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NONE}>{t("noCompare")}</SelectItem>
              {reports
                .filter((r) => r.id !== currentId)
                .map((r) => (
                  <SelectItem key={r.id} value={r.id}>
                    {reportLabel(r.id)}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>
        </div>
        {m && (
          <p className="mb-1.5 text-xs text-muted-foreground">
            {t("scope", {
              pages: m.pages,
              routes: m.routes.length,
              locales: m.locales.length,
              viewports: m.viewports.length,
            })}
          </p>
        )}
      </div>

      {stale && (
        <div
          role="note"
          className="mb-4 flex gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm"
        >
          <TriangleAlert
            className="mt-0.5 size-4 shrink-0 text-warning-foreground"
            aria-hidden
          />
          <p>
            {t("stale", { used: opts.sourceLocale, source: src })}{" "}
            <button
              type="button"
              className="font-medium underline underline-offset-4"
              onClick={() => setShowRun(true)}
            >
              {t("run.button")}
            </button>
          </p>
        </div>
      )}

      {/* Headline: what needs a human, and what moved since the compared report. */}
      <div
        className="mb-3 flex flex-wrap items-baseline gap-x-4 gap-y-1"
        aria-live="polite"
      >
        <p className="text-lg font-semibold">
          {t("toReview", {
            count: all.filter((i) => i.kind !== "clamp").length,
          })}
        </p>
        {b.data && (
          <p className="text-sm">
            <span
              className={cn(
                "font-medium",
                added ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {t("newCount", { count: added })}
            </span>
            {" · "}
            <span
              className={cn(
                "font-medium",
                fixed ? "text-success-foreground" : "text-muted-foreground",
              )}
            >
              {t("fixedCount", { count: fixed })}
            </span>
            <span className="text-muted-foreground">
              {" "}
              {t("since", {
                date: format.dateTime(new Date(b.data.created_at), {
                  dateStyle: "medium",
                }),
              })}
            </span>
          </p>
        )}
      </div>

      <div className="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        {VISUAL_KINDS.map((k) => {
          const n = counted(a.data).filter((i) => i.kind === k).length;
          const on = kinds.includes(k);
          return (
            <button
              key={k}
              type="button"
              aria-pressed={on}
              onClick={() =>
                setKinds(on ? kinds.filter((x) => x !== k) : [...kinds, k])
              }
              className={cn(
                "rounded-xl border p-3 text-start transition-colors hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                on && "border-foreground/40 bg-accent/60",
              )}
            >
              <span
                className={cn(
                  "rounded px-1.5 py-0.5 text-[11px] font-medium",
                  KIND_STYLE[k],
                )}
              >
                {t(`kinds.${k}`)}
              </span>
              <span
                className={cn(
                  "mt-2 block text-2xl font-semibold tabular-nums",
                  n === 0 && "text-muted-foreground/60",
                )}
              >
                {format.number(n)}
              </span>
              <span className="mt-0.5 block text-xs text-muted-foreground">
                {t(`kindHelp.${k}`)}
              </span>
            </button>
          );
        })}
      </div>

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <FilterSelect
          value={locale}
          onChange={setLocale}
          label={t("locale")}
          all={t("allLocales")}
          options={m?.locales ?? []}
        />
        <FilterSelect
          value={route}
          onChange={setRoute}
          label={t("route")}
          all={t("allRoutes")}
          options={m?.routes ?? []}
          mono
        />
        <FilterSelect
          value={viewport}
          onChange={setViewport}
          label={t("viewport")}
          all={t("allViewports")}
          options={m?.viewports ?? []}
        />
        <div className="flex items-center gap-2 px-1">
          <Switch
            id="vqa-expected"
            checked={showExpected}
            onCheckedChange={setShowExpected}
          />
          <Label htmlFor="vqa-expected" className="text-sm font-normal">
            {t("showExpected", { count: hidden })}
          </Label>
        </div>
        {b.data && (
          <div className="flex items-center gap-2 px-1">
            <Switch
              id="vqa-new"
              checked={onlyNew}
              onCheckedChange={setOnlyNew}
            />
            <Label htmlFor="vqa-new" className="text-sm font-normal">
              {t("onlyNew")}
            </Label>
          </div>
        )}
        {(kinds.length > 0 ||
          locale !== ALL ||
          route !== ALL ||
          viewport !== ALL ||
          onlyNew) && (
          <button
            type="button"
            className="text-xs text-muted-foreground underline-offset-2 hover:underline"
            onClick={resetFilters}
          >
            {t("clearFilters")}
          </button>
        )}
      </div>

      {a.isPending ? (
        <Skeleton className="h-96 rounded-xl" />
      ) : a.isError ? (
        <ErrorState error={a.error} onRetry={() => a.refetch()} />
      ) : (
        <Tabs defaultValue="issues">
          <TabsList>
            <TabsTrigger value="issues">
              {t("tabs.issues", { count: visible.length })}
            </TabsTrigger>
            <TabsTrigger value="screens">
              {t("tabs.screens", { count: pages.length })}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="issues" className="mt-2">
            {groups.length === 0 ? (
              all.length === 0 && !onlyNew && !kinds.length ? (
                <EmptyState
                  icon={CircleCheck}
                  title={t("clear.title")}
                  body={t("clear.body", { count: pages.length })}
                />
              ) : (
                <EmptyState
                  icon={ScanSearch}
                  title={t("noMatch")}
                  action={
                    <Button variant="outline" size="sm" onClick={resetFilters}>
                      {t("clearFilters")}
                    </Button>
                  }
                />
              )
            ) : (
              <div className="space-y-4">
                {groups.map((xs) => (
                  <IssueGroup
                    key={`${xs[0].route}|${xs[0].locale}`}
                    slug={p.slug}
                    report={a.data!}
                    issues={xs}
                    isNew={isNew}
                    onShot={(i) => {
                      const pg = pageOf(i);
                      if (pg) setOpen({ page: pg, focus: i });
                    }}
                    onKey={toKey}
                  />
                ))}
              </div>
            )}
          </TabsContent>
          <TabsContent value="screens" className="mt-2">
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
              {pages.map((pg) => (
                <Thumb
                  key={pageId(pg)}
                  slug={p.slug}
                  report={a.data!}
                  page={pg}
                  issues={issuesOf(a.data, pg)}
                  onOpen={() => setOpen({ page: pg })}
                />
              ))}
            </div>
          </TabsContent>
        </Tabs>
      )}

      <Dialog open={!!open} onOpenChange={(o) => !o && setOpen(null)}>
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-6xl">
          {open && a.data && (
            <>
              <DialogHeader>
                <DialogTitle className="flex flex-wrap items-center gap-2">
                  <span className="rounded border px-1.5 font-mono text-xs">
                    {open.page.locale}
                  </span>
                  <span className="font-mono text-sm">{open.page.route}</span>
                  <span className="text-xs font-normal text-muted-foreground">
                    {open.page.viewport}
                  </span>
                </DialogTitle>
                <DialogDescription
                  dir="ltr"
                  className="truncate font-mono text-xs"
                >
                  {open.page.url}
                </DialogDescription>
              </DialogHeader>
              <div className={cn("grid gap-4", b.data && "md:grid-cols-2")}>
                {b.data && (
                  <Shot
                    title={t("before")}
                    slug={p.slug}
                    report={b.data}
                    page={b.data.report.pages.find(
                      (x) => pageId(x) === pageId(open.page),
                    )}
                    issues={issuesOf(b.data, open.page)}
                    onIssue={toKey}
                  />
                )}
                <Shot
                  title={b.data ? t("after") : undefined}
                  slug={p.slug}
                  report={a.data}
                  page={open.page}
                  issues={issuesOf(a.data, open.page)}
                  focus={open.focus}
                  onIssue={toKey}
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

function FilterSelect({
  value,
  onChange,
  label,
  all,
  options,
  mono,
}: {
  value: string;
  onChange: (v: string) => void;
  label: string;
  all: string;
  options: string[];
  mono?: boolean;
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger
        size="sm"
        className={cn("w-40", mono && "font-mono")}
        aria-label={label}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL}>{all}</SelectItem>
        {options.map((o) => (
          <SelectItem key={o} value={o} className={cn(mono && "font-mono")}>
            {o}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function IssueGroup({
  slug,
  report,
  issues,
  isNew,
  onShot,
  onKey,
}: {
  slug: string;
  report: VisualReport;
  issues: VisualIssue[];
  isNew: (i: VisualIssue) => boolean;
  onShot: (i: VisualIssue) => void;
  onKey: (i: VisualIssue) => void;
}) {
  const t = useTranslations("visual");
  const localeName = useLocaleName();
  const { route, locale } = issues[0];
  return (
    <section className="overflow-hidden rounded-xl border">
      <h2 className="flex flex-wrap items-center gap-2 border-b bg-muted/30 px-3 py-2 text-sm font-medium">
        <span dir="ltr" className="font-mono">
          {route}
        </span>
        <span className="rounded border px-1.5 font-mono text-xs">
          {locale}
        </span>
        <span className="text-muted-foreground capitalize">
          {localeName(locale)}
        </span>
        <span className="ms-auto text-xs font-normal text-muted-foreground">
          {t("issues", { count: issues.length })}
        </span>
      </h2>
      <ul className="divide-y">
        {issues.map((i, n) => {
          const Icon = parseViewport(i.viewport).w < 900 ? Smartphone : Monitor;
          return (
            <li
              key={n}
              className="flex flex-col gap-3 px-3 py-2.5 sm:flex-row sm:items-center"
            >
              {i.crop && (
                // eslint-disable-next-line @next/next/no-img-element -- authenticated API image
                <img
                  src={api.visualFileUrl(slug, report.id, i.crop)}
                  alt={t("cropAlt", { key: issueLabel(i) })}
                  loading="lazy"
                  className="h-14 max-w-full shrink-0 rounded border bg-muted/40 object-contain object-left sm:w-56"
                />
              )}
              <div className="min-w-0 flex-1 space-y-0.5">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span
                    className={cn(
                      "rounded px-1.5 py-0.5 text-[11px] font-medium",
                      KIND_STYLE[i.kind],
                    )}
                  >
                    {t(`kinds.${i.kind}`)}
                  </span>
                  {isNew(i) && (
                    <span className="rounded bg-destructive px-1.5 py-0.5 text-[11px] font-medium text-white">
                      {t("new")}
                    </span>
                  )}
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Icon className="size-3" aria-hidden />
                    {i.viewport}
                  </span>
                </div>
                <p dir="ltr" className="truncate text-start font-mono text-xs">
                  {issueLabel(i)}
                </p>
                <p
                  dir="auto"
                  className="line-clamp-2 text-xs text-muted-foreground"
                >
                  {i.text}
                </p>
                {i.suggestion && (
                  <p dir="auto" className="text-xs text-success-foreground">
                    → {i.suggestion}
                  </p>
                )}
              </div>
              <div className="flex shrink-0 gap-1.5">
                <Button variant="outline" size="sm" onClick={() => onShot(i)}>
                  <Images aria-hidden />
                  {t("showOnScreen")}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => onKey(i)}>
                  <KeyRound aria-hidden />
                  {t("openKey")}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Thumb({
  slug,
  report,
  page,
  issues,
  onOpen,
}: {
  slug: string;
  report: VisualReport;
  page: VisualPage;
  issues: VisualIssue[];
  onOpen: () => void;
}) {
  const t = useTranslations("visual");
  const { w } = parseViewport(page.viewport);
  const Icon = w < 900 ? Smartphone : Monitor;
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
          className={cn(
            "w-full object-cover object-top transition-transform duration-300 group-hover:scale-[1.02]",
            w < 900 && "mx-auto w-1/2",
          )}
        />
      </div>
      <div className="flex items-center gap-2 px-3 py-2 text-sm">
        <span className="rounded border px-1 font-mono text-[11px]">
          {page.locale}
        </span>
        <Icon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <span dir="ltr" className="truncate font-mono text-xs">
          {page.route}
        </span>
        <span className="ms-auto shrink-0 text-xs">
          {issues.length === 0 ? (
            <span className="text-success-foreground">{t("clean")}</span>
          ) : (
            <span className="text-destructive">
              {t("issues", { count: issues.length })}
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
  focus,
  onIssue,
}: {
  title?: string;
  slug: string;
  report: VisualReport;
  page?: VisualPage;
  issues: VisualIssue[];
  focus?: VisualIssue;
  onIssue: (i: VisualIssue) => void;
}) {
  const t = useTranslations("visual");
  const format = useFormatter();
  const [cssHeight, setCssHeight] = useState<number | null>(null);
  const { w } = parseViewport(page?.viewport ?? "");
  const focused = (i: VisualIssue) =>
    !!focus && issueSig(i) === issueSig(focus);
  return (
    <div
      className={cn(
        "min-w-0 gap-3",
        title
          ? "space-y-3"
          : "grid lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start",
      )}
    >
      {title && (
        <p className="text-xs font-medium text-muted-foreground">
          {title} ·{" "}
          {format.dateTime(new Date(report.created_at), {
            dateStyle: "medium",
            timeStyle: "short",
          })}
        </p>
      )}
      {!page ? (
        <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
          {t("notInReport")}
        </p>
      ) : (
        <div
          className="max-h-[70dvh] overflow-y-auto rounded-lg border"
          dir="ltr"
        >
          <div className="relative">
            {/* eslint-disable-next-line @next/next/no-img-element -- authenticated API image */}
            <img
              src={api.visualFileUrl(slug, report.id, page.screenshot)}
              alt={t("screenshotAlt", {
                locale: page.locale,
                route: page.route,
              })}
              className="w-full"
              onLoad={(e) =>
                setCssHeight(
                  (e.currentTarget.naturalHeight * w) /
                    e.currentTarget.naturalWidth,
                )
              }
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
                        ref={
                          focused(i)
                            ? (el) => el?.scrollIntoView({ block: "center" })
                            : undefined
                        }
                        onClick={() => onIssue(i)}
                        aria-label={t("issueAt", {
                          kind: t(`kinds.${i.kind}`),
                          key: issueLabel(i),
                        })}
                        className={cn(
                          "absolute rounded-sm bg-destructive/10 outline-2 outline-destructive hover:bg-destructive/25",
                          focused(i) &&
                            "bg-warning/25 outline-4 outline-warning motion-safe:animate-pulse",
                        )}
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
        <ul
          className={cn(
            "divide-y rounded-lg border",
            !title && "lg:max-h-[70dvh] lg:overflow-y-auto",
          )}
        >
          {issues.map((i, n) => (
            <li key={n}>
              <button
                type="button"
                onClick={() => onIssue(i)}
                className={cn(
                  "flex w-full items-start gap-3 px-3 py-2 text-start hover:bg-accent/40",
                  focused(i) && "bg-warning/10",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium",
                    KIND_STYLE[i.kind],
                  )}
                >
                  {t(`kinds.${i.kind}`)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-xs">
                    {issueLabel(i)}
                  </span>
                  <span
                    dir="auto"
                    className="block truncate text-xs text-muted-foreground"
                  >
                    {i.text}
                  </span>
                  {i.suggestion && (
                    <span
                      dir="auto"
                      className="mt-0.5 block text-xs text-success-foreground"
                    >
                      → {i.suggestion}
                    </span>
                  )}
                </span>
                {i.crop && (
                  // eslint-disable-next-line @next/next/no-img-element -- authenticated API image
                  <img
                    src={api.visualFileUrl(slug, report.id, i.crop)}
                    alt=""
                    loading="lazy"
                    className="h-10 max-w-40 shrink-0 rounded border object-contain object-left"
                  />
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
