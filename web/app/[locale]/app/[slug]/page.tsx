"use client";

import { useQuery } from "@tanstack/react-query";
import { CircleAlert, CircleX, Inbox, Languages, Loader2, Sparkles, TableProperties, Wallet } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { EmptyState, ErrorState, PageHeader, ProgressBar, useLocaleName } from "@/components/common";
import { isActive, JobRow } from "@/components/job-row";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk } from "@/lib/api";
import { useProject, useSlug, useTranslateMissing } from "@/lib/hooks";

export default function OverviewPage() {
  const t = useTranslations("overview");
  const format = useFormatter();
  const localeName = useLocaleName();
  const slug = useSlug();
  const project = useProject(slug);
  const issues = useQuery({ queryKey: qk.issues(slug), queryFn: () => api.issues(slug) });
  const jobs = useQuery({
    queryKey: qk.jobs(slug),
    queryFn: () => api.jobs(slug),
    refetchInterval: (q) => (q.state.data?.some(isActive) ? 5000 : false),
  });
  const translate = useTranslateMissing(slug);

  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <OverviewSkeleton />;

  const p = project.data;
  const spent = jobs.data?.reduce((s, j) => s + j.cost_usd, 0) ?? 0;
  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD" });
  const busy = translate.isPending || !!jobs.data?.some(isActive);

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title={p.name}
        subtitle={<span className="font-mono">{p.slug}</span>}
        actions={
          <Button onClick={() => translate.mutate()} disabled={busy || p.stats.keys === 0}>
            {busy ? <Loader2 className="animate-spin" /> : <Sparkles />}
            {t("translateMissing")}
          </Button>
        }
      />

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <Stat icon={TableProperties} label={t("keys")} value={format.number(p.stats.keys)} />
        <Stat icon={Languages} label={t("locales")} value={format.number(p.target_locales.length)} hint={p.target_locales.join(" · ")} />
        <Stat
          icon={Wallet}
          label={t("cost")}
          value={usd(spent)}
          hint={p.llm.budget_usd ? t("budget", { budget: usd(p.llm.budget_usd) }) : t("noBudget")}
        />
        <Stat icon={Sparkles} label={t("model")} value={p.llm.model} hint={p.llm.fallback_model ?? undefined} mono />
      </div>

      {p.stats.keys === 0 ? (
        <div className="mt-6">
          <EmptyState icon={Inbox} title={t("noKeys.title")} body={t("noKeys.body")} />
        </div>
      ) : (
        <div className="mt-6 grid gap-6 lg:grid-cols-5">
          <section className="rounded-xl border lg:col-span-3">
            <h2 className="border-b px-4 py-3 text-sm font-medium">{t("progress")}</h2>
            <ul className="divide-y">
              {p.target_locales.map((l) => {
                const s = p.stats.locales[l] ?? { translated: 0, reviewed: 0, stale: 0, issues: 0 };
                return (
                  <li key={l}>
                    <Link href={`/app/${slug}/keys?locale=${l}`} className="block px-4 py-3 hover:bg-accent/40">
                      <div className="mb-2 flex items-center gap-2 text-sm">
                        <span className="font-mono text-xs">{l}</span>
                        <span className="text-muted-foreground capitalize">{localeName(l)}</span>
                        <span className="ms-auto flex items-center gap-3 text-xs text-muted-foreground">
                          {s.stale > 0 && <span className="text-warning">{t("staleCount", { count: s.stale })}</span>}
                          {s.issues > 0 && <span className="text-destructive">{t("issuesCount", { count: s.issues })}</span>}
                          <span className="font-mono tabular-nums">
                            {format.number(p.stats.keys ? s.translated / p.stats.keys : 0, { style: "percent" })}
                          </span>
                        </span>
                      </div>
                      <ProgressBar total={p.stats.keys} translated={s.translated} reviewed={s.reviewed} stale={s.stale} />
                      <p className="mt-1.5 text-xs text-muted-foreground">
                        {t("progressHint", {
                          translated: format.number(s.translated),
                          total: format.number(p.stats.keys),
                          reviewed: format.number(s.reviewed),
                        })}
                      </p>
                    </Link>
                  </li>
                );
              })}
            </ul>
          </section>

          <section className="rounded-xl border lg:col-span-2">
            <div className="flex items-center border-b px-4 py-3">
              <h2 className="text-sm font-medium">{t("openIssues")}</h2>
              {issues.data && (
                <span className="ms-2 rounded-full bg-muted px-1.5 font-mono text-[11px]">{format.number(issues.data.length)}</span>
              )}
            </div>
            {issues.isPending ? (
              <ListSkeleton />
            ) : issues.isError ? (
              <div className="p-4">
                <ErrorState error={issues.error} onRetry={() => issues.refetch()} />
              </div>
            ) : issues.data.length === 0 ? (
              <p className="px-4 py-8 text-center text-sm text-muted-foreground">{t("noIssues")}</p>
            ) : (
              <ul className="max-h-96 divide-y overflow-y-auto">
                {issues.data.slice(0, 50).map((i) => (
                  <li key={`${i.key}:${i.locale}:${i.issue.code}`}>
                    <Link
                      href={`/app/${slug}/keys?q=${encodeURIComponent(i.key)}&locale=${i.locale}`}
                      className="flex gap-2.5 px-4 py-2.5 hover:bg-accent/40"
                    >
                      {i.issue.severity === "error" ? (
                        <CircleX className="mt-0.5 size-4 shrink-0 text-destructive" />
                      ) : (
                        <CircleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
                      )}
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="truncate font-mono text-xs">{i.key}</span>
                          <span className="shrink-0 rounded border px-1 font-mono text-[10px] text-muted-foreground">{i.locale}</span>
                        </div>
                        <p className="mt-0.5 text-xs text-muted-foreground">{i.issue.message}</p>
                      </div>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      )}

      <section className="mt-6 rounded-xl border">
        <div className="flex items-center justify-between border-b px-4 py-3">
          <h2 className="text-sm font-medium">{t("recentJobs")}</h2>
          <Button variant="ghost" size="xs" asChild>
            <Link href={`/app/${slug}/jobs`}>{t("viewAll")}</Link>
          </Button>
        </div>
        {jobs.isPending ? (
          <ListSkeleton />
        ) : jobs.data?.length ? (
          <div className="divide-y">
            {jobs.data.slice(0, 5).map((j) => (
              <JobRow key={j.id} slug={slug} job={j} compact />
            ))}
          </div>
        ) : (
          <p className="px-4 py-8 text-center text-sm text-muted-foreground">{t("noJobs")}</p>
        )}
      </section>
    </div>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
  hint,
  mono,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
  hint?: string;
  mono?: boolean;
}) {
  return (
    <div className="rounded-xl border p-4">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="size-3.5" /> {label}
      </div>
      <div className={mono ? "mt-2 truncate font-mono text-sm font-medium" : "mt-1.5 text-2xl font-semibold tracking-tight tabular-nums"}>{value}</div>
      {hint && <div className="mt-1 truncate text-xs text-muted-foreground">{hint}</div>}
    </div>
  );
}

function ListSkeleton() {
  return (
    <div className="space-y-3 p-4">
      {Array.from({ length: 4 }, (_, i) => (
        <Skeleton key={i} className="h-8" />
      ))}
    </div>
  );
}

function OverviewSkeleton() {
  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <Skeleton className="h-10 w-64" />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 4 }, (_, i) => (
          <Skeleton key={i} className="h-24 rounded-xl" />
        ))}
      </div>
      <Skeleton className="h-72 rounded-xl" />
    </div>
  );
}
