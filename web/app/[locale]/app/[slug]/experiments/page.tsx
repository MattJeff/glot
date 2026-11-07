"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Archive, FlaskConical, Trophy, Users } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cellViews, CellStateBadge, useXpFormat } from "@/components/experiments/results/shared";
import { useVerdict, XpStatusBadge } from "@/components/experiments/results/summary";
import { isEnded, isLive, XpActionsMenu } from "@/components/experiments/lifecycle";
import { Milestone, MilestoneList } from "@/components/gamification/milestone";
import { Link } from "@/i18n/navigation";
import { api, qk, type VariantExperiment, type XpExperiment } from "@/lib/api";
import { useSlug } from "@/lib/hooks";
import { cn } from "@/lib/utils";
import { ExperimentCard } from "./parts";

const FILTERS = {
  all: (e: XpExperiment) => !e.archived_at,
  draft: (e: XpExperiment) => e.status === "draft",
  live: isLive,
  ended: (e: XpExperiment) => isEnded(e) && !e.archived_at,
  archived: (e: XpExperiment) => !!e.archived_at,
} as const;
type Filter = keyof typeof FILTERS;

// Integration snippet shown as code, not UI copy.
const SNIPPET = `// app/[locale]/layout.tsx — inside NextIntlClientProvider
import { GlotVariants } from "@glot-i18n/runtime/next-intl";
<GlotVariants>{children}</GlotVariants>

// anywhere: the conversion that decides the winner
import { track } from "@glot-i18n/insights";
track("signup");`;

export default function ExperimentsPage() {
  const t = useTranslations("experiments");
  const format = useFormatter();
  const name = useLocaleName();
  const slug = useSlug();
  const qc = useQueryClient();
  const tl = useTranslations("xpLifecycle");
  const [filter, setFilter] = useState<Filter>("all");
  const q = useQuery({ queryKey: qk.variants(slug), queryFn: () => api.variantExperiments(slug), refetchInterval: 30_000 });
  const xq = useQuery({ queryKey: qk.xpList(slug), queryFn: () => api.xpList(slug), refetchInterval: 30_000 });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "promote" | "stop" }) => api.variantAction(slug, id, action),
    onSuccess: (_, v) => {
      qc.invalidateQueries({ queryKey: qk.variants(slug) });
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      toast.success(t(v.action === "promote" ? "promotedToast" : "stoppedToast"));
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (xq.isError) return <ErrorState error={xq.error} onRetry={() => xq.refetch()} />;
  // Segmented experiments (key groups × segments, /xp) get a results page; legacy single-key ones keep their inline card.
  const all = q.data && xq.data ? [...xq.data, ...q.data] : undefined;
  const xps = xq.data ?? [];
  const legacy = q.data ?? [];
  const xpLive = xps.filter((e) => ["draft", "running", "paused", "personalized"].includes(e.status));
  const shown = xps.filter(FILTERS[filter]);
  const live = legacy.filter((e) => e.status === "running" || e.status === "decided");
  const past = legacy.filter((e) => e.status === "promoted" || e.status === "stopped");
  const byLocale = Object.entries(
    live.reduce<Record<string, VariantExperiment[]>>((m, e) => ((m[e.locale] ??= []).push(e), m), {}),
  ).sort(([a], [b]) => a.localeCompare(b));
  const toPromote =
    live.filter((e) => e.status === "decided" && e.winner !== 0).length +
    xpLive.reduce((n, e) => n + e.state.filter((c) => c.winner && !c.personalized).length, 0);
  const liveLocales = new Set([...live.map((e) => e.locale), ...xpLive.flatMap((e) => e.locales)]).size;
  const visitors = live.reduce((n, e) => n + e.arms.reduce((s, a) => s + a.views, 0), 0);
  const anyWinner =
    legacy.some((e) => e.winner != null && e.winner !== 0 && e.status !== "running") ||
    xps.some((e) => e.state.some((c) => c.winner));
  const newButton = (
    <Button asChild>
      <Link href={`/app/${slug}/experiments/new`}>
        <FlaskConical aria-hidden /> {t("new")}
      </Link>
    </Button>
  );

  const card = (e: VariantExperiment) => (
    <ExperimentCard
      key={e.id}
      exp={e}
      busy={act.isPending && act.variables?.id === e.id}
      onAction={(action) => act.mutate({ id: e.id, action })}
    />
  );

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={newButton}
      />

      {!all ? (
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-24 rounded-xl" />
            ))}
          </div>
          <Skeleton className="h-96 rounded-xl" />
        </div>
      ) : all.length === 0 ? (
        <>
          <div className="flex flex-col items-center rounded-xl border border-dashed px-6 py-10 text-center">
            <div className="mb-3 grid size-10 place-items-center rounded-lg border bg-muted/50">
              <FlaskConical className="size-5 text-muted-foreground" aria-hidden />
            </div>
            <h2 className="font-medium">{t("empty.title")}</h2>
            <p className="mt-1 max-w-lg text-sm text-muted-foreground">{t("empty.body")}</p>
            <div className="mt-4">{newButton}</div>
          </div>
          <CodeBlock code={SNIPPET} label={t("install")} />
        </>
      ) : (
        <>
          <div className={cn("grid grid-cols-1 gap-3", live.length ? "sm:grid-cols-3" : "sm:grid-cols-2")}>
            <Stat icon={FlaskConical} label={t("stats.live")} value={format.number(live.length + xpLive.length)} hint={t("stats.liveHint", { n: liveLocales })} />
            <Stat icon={Trophy} label={t("stats.toPromote")} value={format.number(toPromote)} hint={t("stats.toPromoteHint")} />
            {/* Per-visitor counts of segmented experiments live on their results page. */}
            {live.length > 0 && <Stat icon={Users} label={t("stats.visitors")} value={format.number(visitors)} hint={t("stats.visitorsHint")} />}
          </div>

          <ExperimentMilestones started={all.length > 0} won={anyWinner} />

          {xps.length > 0 && (
            <section aria-labelledby="xp-live" className="space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 id="xp-live" className="font-medium">
                  {t("segmented")}
                </h2>
                <div role="group" aria-label={tl("filter.label")} className="flex flex-wrap gap-1.5">
                  {(Object.keys(FILTERS) as Filter[]).map((f) => (
                    <button
                      key={f}
                      type="button"
                      aria-pressed={filter === f}
                      onClick={() => setFilter(f)}
                      className={cn(
                        "min-h-8 rounded-full border px-3 text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                        filter === f && "border-brand bg-brand/10 font-medium",
                      )}
                    >
                      {tl(`filter.${f}`)} <span className="text-xs text-muted-foreground tabular-nums">{xps.filter(FILTERS[f]).length}</span>
                    </button>
                  ))}
                </div>
              </div>
              {shown.length ? (
                <ul className="space-y-3">
                  {shown.map((e) => (
                    <XpRow key={e.id} exp={e} slug={slug} />
                  ))}
                </ul>
              ) : (
                <p className="rounded-xl border border-dashed p-4 text-sm text-muted-foreground">{tl("filter.empty")}</p>
              )}
            </section>
          )}

          {byLocale.map(([locale, exps]) => (
            <section key={locale} aria-labelledby={`loc-${locale}`} className="space-y-3">
              <h2 id={`loc-${locale}`} className="flex items-center gap-2 font-medium">
                {name(locale)} <span className="font-mono text-xs text-muted-foreground">{locale}</span>
              </h2>
              {exps.map(card)}
            </section>
          ))}

          {past.length > 0 && (
            <details className="group rounded-xl border p-4 sm:p-5">
              <summary className="cursor-pointer font-medium">{t("history", { n: past.length })}</summary>
              <div className="mt-4 space-y-3">{past.map(card)}</div>
            </details>
          )}
          <CodeBlock code={SNIPPET} label={t("install")} />
        </>
      )}

    </div>
  );
}

function Stat({ icon: Icon, label, value, hint }: { icon: React.ComponentType<{ className?: string }>; label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border p-4">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="size-3.5" /> {label}
      </div>
      <div className="mt-1.5 text-2xl font-semibold tracking-tight tabular-nums">{value}</div>
      {hint && <div className="mt-1 truncate text-xs text-muted-foreground">{hint}</div>}
    </div>
  );
}

/** One segmented experiment in the list: status, name, the verdict sentence, each segment's state. */
function XpRow({ exp, slug }: { exp: XpExperiment; slug: string }) {
  const t = useTranslations("experimentResults");
  const tl = useTranslations("xpLifecycle");
  const { cellName } = useXpFormat();
  const cells = cellViews(exp, null);
  const verdict = useVerdict(exp, cells);
  return (
    <li className="relative">
      {/* Menu sits outside the link (no nested interactive content); the link keeps room for it. */}
      <div className="absolute end-3 top-3 z-10 sm:end-4 sm:top-4">
        <XpActionsMenu exp={exp} slug={slug} />
      </div>
      <Link
        href={`/app/${slug}/experiments/${exp.id}`}
        className="group flex items-start gap-3 rounded-xl border bg-card p-4 pe-14 shadow-(--shadow-card) transition-colors hover:bg-accent/40 sm:p-5 sm:pe-16"
      >
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-medium">{exp.name}</h3>
            <XpStatusBadge status={exp.status} />
            {exp.archived_at && (
              <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                <Archive className="size-3" aria-hidden /> {tl("archivedBadge")}
              </span>
            )}
          </div>
          <p className="mt-1 text-sm text-pretty text-muted-foreground">{verdict}</p>
          <ul className="mt-3 flex flex-wrap gap-x-4 gap-y-2 text-xs" aria-label={t("list.segments")}>
            {cells.map((c) => (
              <li key={c.id} className="flex items-center gap-1.5">
                <span className="text-muted-foreground">{cellName(c, cells.length === 1)}</span>
                <CellStateBadge state={c.state} />
              </li>
            ))}
          </ul>
          <p className="mt-2 text-xs text-muted-foreground tabular-nums">
            {t("list.meta", { keys: exp.keys.length, locales: exp.locales.length })}
          </p>
        </div>
      </Link>
    </li>
  );
}

/** Two real milestones (ux-brief §2): no points, just what the team actually achieved. */
function ExperimentMilestones({ started, won }: { started: boolean; won: boolean }) {
  const t = useTranslations("experimentResults.milestones");
  return (
    <MilestoneList label={t("label")} className="rounded-xl border p-1">
      <Milestone icon={FlaskConical} title={t("first")} description={t("firstBody")} achieved={started} />
      <Milestone icon={Trophy} title={t("winner")} description={t("winnerBody")} achieved={won} />
    </MilestoneList>
  );
}
