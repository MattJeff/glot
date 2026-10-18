"use client";

import { useQuery } from "@tanstack/react-query";
import { ListChecks, Loader2, Sparkles } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { EmptyState, ErrorState, PageHeader } from "@/components/common";
import { isActive, JobRow } from "@/components/job-row";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api, qk } from "@/lib/api";
import { useSlug, useTranslateMissing } from "@/lib/hooks";

export default function JobsPage() {
  const t = useTranslations("jobs");
  const to = useTranslations("overview");
  const format = useFormatter();
  const slug = useSlug();
  const jobs = useQuery({
    queryKey: qk.jobs(slug),
    queryFn: () => api.jobs(slug),
    // Fallback polling if the SSE stream drops.
    refetchInterval: (q) => (q.state.data?.some(isActive) ? 5000 : false),
  });
  const translate = useTranslateMissing(slug);
  const busy = translate.isPending || !!jobs.data?.some(isActive);
  const spent = jobs.data?.reduce((s, j) => s + j.cost_usd, 0) ?? 0;

  const button = (
    <Button onClick={() => translate.mutate()} disabled={busy}>
      {busy ? <Loader2 className="animate-spin" /> : <Sparkles />}
      {to("translateMissing")}
    </Button>
  );

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title={t("title")}
        subtitle={
          <>
            {t("subtitle")}{" "}
            {jobs.data?.length ? (
              <span className="font-mono">
                {t("cost")}: {format.number(spent, { style: "currency", currency: "USD" })}
              </span>
            ) : null}
          </>
        }
        actions={button}
      />
      {jobs.isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-16 rounded-lg" />
          ))}
        </div>
      ) : jobs.isError ? (
        <ErrorState error={jobs.error} onRetry={() => jobs.refetch()} />
      ) : jobs.data.length === 0 ? (
        <EmptyState icon={ListChecks} title={t("empty.title")} body={t("empty.body")} action={button} />
      ) : (
        <ul className="stagger divide-y overflow-hidden rounded-xl border bg-card shadow-(--shadow-card)" aria-label={t("title")}>
          {jobs.data.map((j, i) => (
            <li key={j.id} style={{ "--i": i } as React.CSSProperties}>
              <JobRow slug={slug} job={j} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
