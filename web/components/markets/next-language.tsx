"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Languages, Plus, Trophy } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { toast } from "sonner";
import { useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api, qk, type Market } from "@/lib/api";

/** "Next language" card (Labs): the best market to add, one click to add it to target_locales. */
export function NextLanguageCard({ slug, days = 30 }: { slug: string; days?: number }) {
  const t = useTranslations("markets");
  const format = useFormatter();
  const name = useLocaleName();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: [...qk.markets(slug), days], queryFn: () => api.markets(slug, days) });
  const project = useQuery({ queryKey: qk.project(slug), queryFn: () => api.project(slug) });
  const add = useMutation({
    mutationFn: (m: Market) =>
      api.updateProject(slug, { target_locales: [...(project.data?.target_locales ?? []), m.locale] }),
    onSuccess: (_, m) => {
      for (const k of [qk.markets(slug), qk.project(slug), qk.insights(slug)]) qc.invalidateQueries({ queryKey: k });
      toast.success(t("added", { language: name(m.locale) }));
    },
    onError: (e) => toast.error(e.message),
  });

  if (q.isError) return null; // ponytail: Labs card, the page around it already reports API errors.
  if (!q.data || !project.data) return <Skeleton className="h-44 rounded-xl" />;
  const [best, ...rest] = q.data.markets;
  if (!best) return null;
  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD", maximumFractionDigits: n > 0 && n < 0.1 ? 4 : 2 });
  const pct = (n: number) => format.number(n, { style: "percent", maximumFractionDigits: n < 0.01 ? 1 : 0 });
  const effort = (m: Market) => (m.effort_notes.length === 0 ? t("noEffort") : m.effort_notes.join(" · "));

  return (
    <section aria-labelledby="next-language" className="rounded-xl border bg-gradient-to-br from-brand/10 via-background to-background p-5 sm:p-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 max-w-2xl">
          <div className="mb-2 flex flex-wrap items-center gap-2">
            <h2 id="next-language" className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Languages className="size-3.5" aria-hidden /> {t("kicker")}
            </h2>
            <Badge>
              <Trophy aria-hidden /> {t("badge")}
            </Badge>
          </div>
          <p className="text-xl font-semibold tracking-tight text-balance sm:text-2xl">
            {t("headline", { language: name(best.locale), locale: best.locale })}
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            {[
              best.visits > 0
                ? t("demand", { share: pct(best.share) })
                : t("public", { users: format.number(best.internet_users_m, { maximumFractionDigits: 0 }) }),
              t("cost", {
                cost: usd(best.cost_usd),
                review:
                  best.review_hours < 1
                    ? t("minutes", { n: Math.max(1, Math.ceil(best.review_hours * 60)) })
                    : t("hours", { n: format.number(best.review_hours, { maximumFractionDigits: 1 }) }),
              }),
              effort(best),
            ].join(" · ")}
          </p>
        </div>
        <div className="flex flex-col items-end gap-2">
          <span className="text-sm tabular-nums" aria-label={t("scoreLabel", { score: best.score })}>
            <span className="text-2xl font-semibold">{best.score}</span>
            <span className="text-muted-foreground">/100</span>
          </span>
          <Button onClick={() => add.mutate(best)} disabled={add.isPending}>
            <Plus aria-hidden /> {t("add", { language: name(best.locale) })}
          </Button>
        </div>
      </div>

      {rest.length > 0 && (
        <ul className="mt-4 grid gap-2 sm:grid-cols-2 lg:grid-cols-3" aria-label={t("runnersUp")}>
          {rest.slice(0, 3).map((m) => (
            <li key={m.locale} className="flex items-center justify-between gap-2 rounded-lg border bg-background/60 px-3 py-2 text-sm">
              <span className="min-w-0">
                <span className="block truncate font-medium">{name(m.locale)}</span>
                <span className="block truncate text-xs text-muted-foreground">
                  {usd(m.cost_usd)} · {effort(m)}
                </span>
              </span>
              <Button
                size="sm"
                variant="outline"
                onClick={() => add.mutate(m)}
                disabled={add.isPending}
                aria-label={t("add", { language: name(m.locale) })}
              >
                <Plus aria-hidden /> {m.score}
              </Button>
            </li>
          ))}
        </ul>
      )}
      <p className="mt-3 text-xs text-muted-foreground">
        {t("footnote", { visits: q.data.total_visits, days: q.data.days, date: q.data.data_as_of })}
      </p>
    </section>
  );
}
