"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FileSearch, Star } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, ProgressBar, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import type { Project } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { type EdgeSettings, edgeApi, edgeKeys } from "./api";

/** Coverage per page actually served (segments rendered translated / rendered), critical pages first. */
export function Coverage({ project, settings }: { project: Project; settings: EdgeSettings }) {
  const t = useTranslations("edge.coverage");
  const format = useFormatter();
  const name = useLocaleName();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const slug = project.slug;
  const coverage = useQuery({ queryKey: edgeKeys.coverage(slug), queryFn: () => edgeApi.coverage(slug) });
  const [picked, setPicked] = useState<string>();
  const current = coverage.data?.locales.find((l) => l.locale === picked) ?? coverage.data?.locales[0];
  const locale = current?.locale ?? "";
  // A page is marked critical by adding its exact path to `critical_patterns` (globs like /pricing* stay as they are).
  const mark = useMutation({
    mutationFn: ({ url, critical }: { url: string; critical: boolean }) =>
      edgeApi.putSettings(slug, {
        ...settings,
        critical_patterns: critical ? [...settings.critical_patterns, url] : settings.critical_patterns.filter((p) => p !== url),
      }),
    onSuccess: (s) => {
      qc.setQueryData(edgeKeys.settings(slug), s);
      qc.invalidateQueries({ queryKey: edgeKeys.all(slug) });
    },
    onError: (e) => toast.error(message(e)),
  });
  const pct = (n: number | null) => (n === null ? "—" : format.number(n, { style: "percent", maximumFractionDigits: 1 }));
  const threshold = current?.critical_threshold ?? settings.critical_threshold;
  const pages = current?.pages ?? []; // server order: critical first, then lowest coverage

  if (coverage.isPending) return <Skeleton className="h-64 rounded-xl" />;
  if (coverage.isError) return <ErrorState error={coverage.error} onRetry={() => coverage.refetch()} />;

  return (
    <div className="space-y-8">
      <p className="text-sm text-muted-foreground">{t("intro", { critical: pct(threshold), global: pct(coverage.data.seo_min) })}</p>
      <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {coverage.data.locales.map((l) => (
          <li key={l.locale}>
            <button
              type="button"
              onClick={() => setPicked(l.locale)}
              aria-pressed={l.locale === locale}
              className={cn("w-full rounded-xl border p-4 text-start transition-colors hover:bg-accent/50", l.locale === locale && "border-brand")}
            >
              <div className="flex items-center gap-2">
                <span className="font-medium">{name(l.locale)}</span>
                <Badge variant="secondary" className={l.ready ? "bg-success/12 text-success-foreground" : "text-muted-foreground"}>
                  {l.ready ? t("ready") : t("notReady")}
                </Badge>
                <span className="ms-auto font-mono text-xs text-muted-foreground">v{l.version}</span>
              </div>
              <ProgressBar className="mt-3" total={1000} translated={Math.round(l.coverage * 1000)} reviewed={Math.round(l.coverage * 1000)} label={t("global")} />
              <dl className="mt-2 grid grid-cols-3 gap-2 text-xs">
                <div>
                  <dt className="text-muted-foreground">{t("global")}</dt>
                  <dd className="tabular-nums">{pct(l.coverage)}</dd>
                </div>
                <div>
                  <dt className="text-muted-foreground">{t("critical")}</dt>
                  <dd className={cn("tabular-nums", l.critical_coverage !== null && l.critical_coverage < l.critical_threshold && "text-warning-foreground")}>
                    {pct(l.critical_coverage)}
                  </dd>
                </div>
                <div>
                  <dt className="text-muted-foreground">{t("pagesSeen")}</dt>
                  <dd className="tabular-nums">{format.number(l.pages.length)}</dd>
                </div>
              </dl>
            </button>
          </li>
        ))}
      </ul>

      {pages.length === 0 ? (
        <EmptyState icon={FileSearch} title={t("empty.title")} body={t("empty.body")} />
      ) : (
        <ul className="divide-y rounded-xl border" aria-label={t("pages", { locale: name(locale) })}>
          {pages.map((pg) => {
            // Critical through a glob: only editable in settings.
            const byGlob = pg.critical && !settings.critical_patterns.includes(pg.path);
            return (
              <li key={pg.path} className="space-y-2 p-4">
                <div className="flex flex-wrap items-center gap-3">
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-pressed={pg.critical}
                    aria-label={pg.critical ? t("unmark") : t("mark")}
                    title={byGlob ? t("byPattern") : pg.critical ? t("unmark") : t("mark")}
                    disabled={byGlob || mark.isPending}
                    onClick={() => mark.mutate({ url: pg.path, critical: !pg.critical })}
                  >
                    <Star className={cn(pg.critical && "fill-warning text-warning")} />
                  </Button>
                  <span dir="ltr" translate="no" className="min-w-0 flex-1 truncate font-mono text-sm">
                    {pg.path}
                  </span>
                  {pg.critical && <Badge variant="outline">{t("criticalBadge")}</Badge>}
                  <span
                    className={cn(
                      "w-16 text-end text-sm tabular-nums",
                      pg.critical && pg.coverage < threshold ? "text-destructive" : pg.coverage < 1 && "text-warning-foreground",
                    )}
                  >
                    {pct(pg.coverage)}
                  </span>
                </div>
                <ProgressBar total={pg.segments} translated={pg.translated} reviewed={pg.translated} label={pg.path} />
                {pg.missing.length > 0 && (
                  <details>
                    <summary className="cursor-pointer text-xs text-muted-foreground">{t("missing", { n: pg.segments - pg.translated })}</summary>
                    <ul className="mt-2 flex flex-wrap gap-1.5">
                      {pg.missing.map((m) => (
                        <li key={m.hash + m.context} hidden={!m.text}>
                          <Link
                            href={`/app/${slug}/keys?view=segments&locale=${locale}&q=${encodeURIComponent(m.text ?? "")}`}
                            className="inline-block max-w-80 truncate rounded-md border px-2 py-0.5 text-xs hover:bg-accent"
                          >
                            {m.text}
                          </Link>
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
