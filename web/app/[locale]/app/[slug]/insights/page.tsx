"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FlaskConical, Globe2, Radar, ShieldCheck, Target, TrendingUp, Users } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { api, qk, type InsightDemand } from "@/lib/api";
import { useSlug } from "@/lib/hooks";
import { DailyChart, DemandTable, ExperimentCard, ExperimentDialog, FunnelPlot, Install, Section, usePct } from "./parts";

const PERIODS = [7, 30, 90];

export default function InsightsPage() {
  const t = useTranslations("insights");
  const format = useFormatter();
  const name = useLocaleName();
  const pct = usePct();
  const slug = useSlug();
  const qc = useQueryClient();
  const [days, setDays] = useState(30);
  const [goal, setGoal] = useState<string | undefined>();
  const [testing, setTesting] = useState<InsightDemand | "new" | null>(null);
  const q = useQuery({
    queryKey: [...qk.insights(slug), days, goal],
    queryFn: () => api.insights(slug, { days, goal }),
  });
  const decide = useMutation({
    mutationFn: ({ id, status }: { id: string; status: "kept" | "removed" }) => api.decideExperiment(slug, id, status),
    onSuccess: (_, v) => {
      qc.invalidateQueries({ queryKey: qk.insights(slug) });
      qc.invalidateQueries({ queryKey: qk.project(slug) });
      toast.success(t(v.status === "kept" ? "exp.keptToast" : "exp.removedToast"));
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const d = q.data;
  const goals = d ? Array.from(new Set(["signup", "purchase", ...d.events])) : ["signup"];

  const header = (
    <PageHeader
      title={t("title")}
      subtitle={t("subtitle")}
      actions={
        <>
          <Select value={goal ?? d?.goal ?? "signup"} onValueChange={setGoal}>
            <SelectTrigger size="sm" className="w-40" aria-label={t("goal")}>
              <Target className="size-3.5" />
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {goals.map((g) => (
                <SelectItem key={g} value={g}>
                  {g}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={String(days)} onValueChange={(v) => setDays(Number(v))}>
            <SelectTrigger size="sm" className="w-36" aria-label={t("period")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PERIODS.map((p) => (
                <SelectItem key={p} value={String(p)}>
                  {t("lastDays", { days: p })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </>
      }
    />
  );

  if (!d)
    return (
      <div className="mx-auto max-w-6xl">
        {header}
        <Skeleton className="h-32 rounded-xl" />
        <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-24 rounded-xl" />
          ))}
        </div>
        <Skeleton className="mt-4 h-80 rounded-xl" />
      </div>
    );

  const gaps = d.demand.filter((x) => !x.offered);
  const top = gaps[0];
  const unserved = gaps.reduce((s, x) => s + x.visits, 0);
  const source = d.funnel.find((f) => f.is_source);
  const running = d.experiments.filter((e) => e.status === "running");
  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD", maximumFractionDigits: n < 10 ? 2 : 0 });

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      {header}

      {d.total_visits === 0 ? (
        <>
          <div className="flex flex-col items-center rounded-xl border border-dashed px-6 py-10 text-center">
            <div className="mb-3 grid size-10 place-items-center rounded-lg border bg-muted/50">
              <Radar className="size-5 text-muted-foreground" />
            </div>
            <h2 className="font-medium">{t("empty.title")}</h2>
            <p className="mt-1 max-w-md text-sm text-muted-foreground">{t("empty.body")}</p>
          </div>
          <Install slug={slug} publicKey={d.public_key} open />
        </>
      ) : (
        <>
          {top ? (
            <div className="relative overflow-hidden rounded-xl border bg-gradient-to-br from-info/10 via-background to-background p-5 sm:p-6">
              <div className="flex flex-wrap items-start justify-between gap-4">
                <div className="min-w-0 max-w-2xl">
                  <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-info">
                    <Globe2 className="size-3.5" /> {t("hero.kicker")}
                  </div>
                  <p className="text-xl font-semibold tracking-tight text-balance sm:text-2xl">
                    {t.rich("hero.headline", {
                      share: pct(top.share, 0),
                      language: name(top.language),
                      strong: (c) => <span className="text-info">{c}</span>,
                    })}
                  </p>
                  <p className="mt-2 text-sm text-muted-foreground">
                    {[
                      top.est_extra_conversions != null &&
                        t("hero.potential", { n: format.number(top.est_extra_conversions, { maximumFractionDigits: 0 }), goal: d.goal }),
                      top.est_cost_usd != null && t("hero.cost", { cost: usd(top.est_cost_usd), keys: d.keys }),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                </div>
                {!running.some((e) => e.locale.split("-")[0] === top.language) && (
                  <Button onClick={() => setTesting(top)}>
                    <FlaskConical /> {t("hero.cta", { language: name(top.language) })}
                  </Button>
                )}
              </div>
            </div>
          ) : (
            <div className="flex items-center gap-3 rounded-xl border bg-success/5 p-5 text-sm">
              <ShieldCheck className="size-5 text-success" /> {t("hero.covered")}
            </div>
          )}

          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Stat icon={Users} label={t("stats.visits")} value={format.number(d.total_visits)} hint={t("lastDays", { days: d.days })} />
            <Stat
              icon={Globe2}
              label={t("stats.unserved")}
              value={pct(unserved / Math.max(1, d.total_visits))}
              hint={t("stats.unservedHint", { n: gaps.length })}
            />
            <Stat
              icon={Target}
              label={t("stats.sourceConv", { goal: d.goal })}
              value={source ? pct(source.rate) : "—"}
              hint={source ? `${name(source.locale)} · n = ${format.number(source.visits)}` : undefined}
            />
            <Stat
              icon={TrendingUp}
              label={t("stats.potential")}
              value={format.number(
                gaps.reduce((s, x) => s + (x.est_extra_conversions ?? 0), 0),
                { maximumFractionDigits: 0 },
              )}
              hint={t("stats.potentialHint", { goal: d.goal })}
            />
          </div>

          <Section title={t("demand.title")} subtitle={t("demand.subtitle")}>
            <DemandTable demand={d.demand} running={running.map((e) => e.locale)} onTest={setTesting} />
          </Section>

          <div className="grid gap-6 lg:grid-cols-5">
            <Section className="lg:col-span-3" title={t("daily.title")} subtitle={t("daily.subtitle")}>
              <DailyChart daily={d.daily} />
            </Section>
            <Section className="lg:col-span-2" title={t("funnel.title", { goal: d.goal })} subtitle={t("funnel.subtitle")}>
              <FunnelPlot funnel={d.funnel} />
            </Section>
          </div>

          <Section
            title={t("exp.title")}
            subtitle={t("exp.subtitle")}
            actions={
              <Button size="sm" variant="outline" onClick={() => setTesting("new")}>
                <FlaskConical /> {t("exp.new")}
              </Button>
            }
          >
            {d.experiments.length === 0 ? (
              <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">{t("exp.empty")}</p>
            ) : (
              <div className="grid gap-3 md:grid-cols-2">
                {d.experiments.map((e) => (
                  <ExperimentCard
                    key={e.id}
                    exp={e}
                    sourceLocale={d.source_locale}
                    busy={decide.isPending && decide.variables?.id === e.id}
                    onDecide={(status) => decide.mutate({ id: e.id, status })}
                  />
                ))}
              </div>
            )}
          </Section>

          <Install slug={slug} publicKey={d.public_key} />
        </>
      )}

      <ExperimentDialog
        key={testing === null || testing === "new" ? String(testing) : testing.language}
        slug={slug}
        target={testing}
        keys={d.keys}
        goal={d.goal}
        onClose={() => setTesting(null)}
      />
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
  hint?: string;
}) {
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

