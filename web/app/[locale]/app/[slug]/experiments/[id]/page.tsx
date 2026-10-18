"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useParams } from "next/navigation";
import { toast } from "sonner";
import { ErrorState } from "@/components/common";
import { CellCard, type Act } from "@/components/experiments/results/cell-card";
import { WordingMatrix } from "@/components/experiments/results/segment-preview";
import { cellViews } from "@/components/experiments/results/shared";
import { SummaryHeader } from "@/components/experiments/results/summary";
import { Skeleton } from "@/components/ui/skeleton";
import { useRouter } from "@/i18n/navigation";
import { api, ApiError, qk } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";

export default function ExperimentResultsPage() {
  const t = useTranslations("experimentResults");
  const slug = useSlug();
  const { id } = useParams<{ id: string }>();
  const qc = useQueryClient();
  const router = useRouter();
  const project = useProject(slug);
  const q = useQuery({ queryKey: qk.xp(slug, id), queryFn: () => api.xp(slug, id), refetchInterval: 30_000 });
  const started = !!q.data?.started_at;
  const res = useQuery({
    queryKey: qk.xpResults(slug, id),
    // Not computed yet (just started, or the stats job never ran): show the experiment without numbers.
    queryFn: () => api.xpResults(slug, id).catch((e) => (e instanceof ApiError && (e.status === 404 || e.status === 409) ? null : Promise.reject(e))),
    enabled: started,
    refetchInterval: 30_000,
  });
  const act = useMutation({
    mutationFn: async (a: Act) => {
      if (a.action !== "propagate") return api.xpAction(slug, id, a);
    },
    onSuccess: (exp, a) => {
      if (a.action === "propagate") {
        // XP9 owns the follow-up test: the wizard picks the winning idea up from the query string.
        router.push(`/app/${slug}/experiments/new?from=${id}&arm=${a.arm}`);
        return;
      }
      if (exp) qc.setQueryData(qk.xp(slug, id), exp);
      qc.invalidateQueries({ queryKey: qk.variants(slug) });
      if (a.action === "promote") qc.invalidateQueries({ queryKey: qk.keys(slug) });
      toast.success(t(a.action === "promote" ? (a.scope === "all" ? "toast.promote_all" : "toast.promote_segment") : `toast.${a.action}`));
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const exp = q.data;
  if (!exp || (started && res.isPending))
    return (
      <div className="mx-auto max-w-6xl space-y-4" aria-busy>
        <Skeleton className="h-16 rounded-xl" />
        <Skeleton className="h-20 rounded-xl" />
        <div className="grid gap-3 sm:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-24 rounded-xl" />
          ))}
        </div>
        <Skeleton className="h-80 rounded-xl" />
      </div>
    );

  const cells = cellViews(exp, res.data);
  const busy = act.isPending;
  const only = cells.length === 1;
  const p = project.data;
  const otherLocales = p ? [p.source_locale, ...p.target_locales].filter((l) => !exp.locales.includes(l)).length : 0;

  return (
    <div className="mx-auto max-w-6xl space-y-8">
      <SummaryHeader exp={exp} cells={cells} slug={slug} busy={busy} onAct={act.mutate} />

      {res.isError && <ErrorState error={res.error} onRetry={() => res.refetch()} />}

      <section aria-labelledby="xp-segments" className="space-y-3">
        <div>
          <h2 id="xp-segments" className="font-medium">
            {t("bySegment")}
          </h2>
          <p className="mt-0.5 text-sm text-pretty text-muted-foreground">{t("bySegmentHint")}</p>
        </div>
        {cells.map((c) => (
          <CellCard key={c.id} exp={exp} cell={c} only={only} otherLocales={otherLocales} busy={busy} onAct={act.mutate} />
        ))}
      </section>

      <section aria-labelledby="xp-wording" className="space-y-3">
        <div>
          <h2 id="xp-wording" className="font-medium">
            {t("wording")}
          </h2>
          <p className="mt-0.5 text-sm text-pretty text-muted-foreground">{t("wordingHint")}</p>
        </div>
        <WordingMatrix slug={slug} exp={exp} cell={cells.find((c) => c.state === "winner") ?? cells[0]} />
      </section>
    </div>
  );
}
