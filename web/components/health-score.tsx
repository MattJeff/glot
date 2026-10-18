"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleCheck, CircleX, Languages, Loader2, TriangleAlert } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useMemo, useState } from "react";
import { useLocaleName } from "@/components/common";
import { seedReviews } from "@/components/review";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Link } from "@/i18n/navigation";
import { api, ApiError, type Project, qk, type Review } from "@/lib/api";

type Dim = "coverage" | "errors" | "meaning" | "visual";
const DIMS: Dim[] = ["coverage", "errors", "meaning", "visual"];
type Cell = { score: number | null; detail: Record<string, number> };

/**
 * Sequential single-hue ramp (brand) over [FLOOR, 100] — real scores cluster high, so the low end is clamped.
 * Capped at 56% so the foreground text token stays readable in both themes.
 */
const FLOOR = 40;
const ramp = (score: number) =>
  `color-mix(in oklch, var(--brand) ${Math.round(6 + (Math.max(0, score - FLOOR) / (100 - FLOOR)) * 50)}%, transparent)`;

function Status({ score }: { score: number }) {
  const t = useTranslations("health.status");
  const [Icon, cls, label] =
    score >= 90 ? [CircleCheck, "text-success", t("good")] : score >= 70 ? [TriangleAlert, "text-warning", t("fair")] : [CircleX, "text-destructive", t("poor")];
  return (
    <span className={`inline-flex items-center gap-1 text-xs ${cls}`}>
      <Icon className="size-3.5" aria-hidden /> {label}
    </span>
  );
}

export function HealthScore({ project: p }: { project: Project }) {
  const t = useTranslations("health");
  const format = useFormatter();
  const localeName = useLocaleName();
  const qc = useQueryClient();
  const report = useQuery({
    queryKey: qk.visualReport(p.slug, "latest"),
    queryFn: () => api.visualReport(p.slug, "latest").catch((e) => (e instanceof ApiError && e.status === 404 ? null : Promise.reject(e))),
  });

  // Meaning comes from reviews already in the query cache (never spends LLM budget on its own).
  const [tick, setTick] = useState(0);
  useEffect(
    () =>
      qc.getQueryCache().subscribe((e) => {
        if (e.type === "updated" && e.query.queryKey[0] === "review" && e.query.queryKey[1] === p.slug) setTick((n) => n + 1);
      }),
    [qc, p.slug],
  );
  const allReviews = useMemo(
    () =>
      qc
        .getQueriesData<Review>({ queryKey: ["review", p.slug] })
        .map(([, r]) => r)
        .filter((r): r is Review => !!r),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- tick: the review cache changed.
    [qc, p.slug, tick],
  );
  const reviewsOf = (l: string) => allReviews.filter((r) => r.locale === l);

  const review = useMutation({
    mutationFn: (l: string) => api.reviewLocale(p.slug, { locale: l }),
    onSuccess: (reviews) => seedReviews(qc, p.slug, reviews),
  });

  const keys = p.stats.keys;
  const rows = p.target_locales.map((l) => {
    const s = p.stats.locales[l] ?? { translated: 0, reviewed: 0, stale: 0, issues: 0 };
    const reviews = reviewsOf(l);
    const pages = report.data?.report.pages.filter((x) => x.locale === l) ?? [];
    // Layout problems only: untranslated text is already counted by coverage/validation.
    const dirty = new Set(
      report.data?.report.issues.filter((i) => i.locale === l && i.kind !== "untranslated").map((i) => `${i.route}|${i.viewport}`),
    );
    const cells: Record<Dim, Cell> = {
      coverage: { score: keys ? (s.translated / keys) * 100 : null, detail: { translated: s.translated, total: keys } },
      errors: { score: s.translated ? (1 - s.issues / s.translated) * 100 : null, detail: { issues: s.issues, translated: s.translated } },
      meaning: {
        score: reviews.length
          ? (reviews.reduce((n, r) => n + (r.verdict === "same" ? 1 : r.verdict === "nuance" ? 0.5 : 0), 0) / reviews.length) * 100
          : null,
        detail: {
          reviewed: reviews.length,
          different: reviews.filter((r) => r.verdict === "different").length,
        },
      },
      visual: {
        score: pages.length ? ((pages.length - dirty.size) / pages.length) * 100 : null,
        detail: { clean: pages.length - dirty.size, pages: pages.length },
      },
    };
    const known = DIMS.map((d) => cells[d].score).filter((x): x is number => x !== null);
    const overall = known.length ? known.reduce((a, b) => a + b, 0) / known.length : null;
    return { locale: l, cells, overall };
  });

  const pct = (n: number) => format.number(Math.round(n));

  return (
    <section className="mt-6 rounded-xl border">
      <div className="flex flex-wrap items-center gap-3 border-b px-4 py-3">
        <h2 className="text-sm font-medium">{t("title")}</h2>
        <p className="text-xs text-muted-foreground">{t("subtitle")}</p>
        <div className="ms-auto flex items-center gap-2 text-[11px] text-muted-foreground" aria-hidden>
          <span>≤{FLOOR}</span>
          <span
            className="h-2 w-28 rounded-full rtl:-scale-x-100"
            style={{ background: `linear-gradient(to right, ${ramp(0)}, ${ramp(100)})` }}
          />
          <span>100</span>
        </div>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full border-separate border-spacing-[3px] text-sm">
          <thead>
            <tr className="text-xs text-muted-foreground">
              <th scope="col" className="px-3 py-2 text-start font-medium">{t("locale")}</th>
              <th scope="col" className="w-44 px-3 py-2 text-start font-medium">{t("overall")}</th>
              {DIMS.map((d) => (
                <th key={d} scope="col" className="w-[14%] px-3 py-2 text-center font-medium">{t(`dims.${d}`)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.locale}>
                <th scope="row" className="px-3 py-1.5 text-start font-normal">
                  <span className="font-mono text-xs">{r.locale}</span>{" "}
                  <span className="text-muted-foreground capitalize">{localeName(r.locale)}</span>
                </th>
                <td className="px-3 py-1.5">
                  {r.overall === null ? (
                    <span className="text-muted-foreground">—</span>
                  ) : (
                    <span className="flex items-center gap-3">
                      <span className="w-8 text-xl font-semibold tracking-tight tabular-nums">{pct(r.overall)}</span>
                      <Status score={r.overall} />
                    </span>
                  )}
                </td>
                {DIMS.map((d) => {
                  const c = r.cells[d];
                  return (
                    <td key={d} className="h-11 min-w-24 p-0">
                      {c.score === null ? (
                        d === "meaning" && (p.stats.locales[r.locale]?.translated ?? 0) > 0 ? (
                          <div className="flex h-full items-center justify-center rounded-md border border-dashed">
                            <Button
                              variant="ghost"
                              size="xs"
                              disabled={review.isPending}
                              onClick={() => review.mutate(r.locale)}
                            >
                              {review.isPending && review.variables === r.locale ? <Loader2 className="animate-spin" /> : <Languages />}
                              {t("runReview")}
                            </Button>
                          </div>
                        ) : (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <div
                                tabIndex={0}
                                className="flex h-full items-center justify-center rounded-md border border-dashed text-xs text-muted-foreground"
                                style={{ backgroundImage: "repeating-linear-gradient(135deg, transparent 0 6px, color-mix(in oklch, var(--muted-foreground) 12%, transparent) 6px 7px)" }}
                              >
                                {t("na")}
                              </div>
                            </TooltipTrigger>
                            <TooltipContent>{t(`empty.${d}`)}</TooltipContent>
                          </Tooltip>
                        )
                      ) : (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Link
                              href={d === "visual" ? `/app/${p.slug}/visual` : d === "meaning" ? `/app/${p.slug}/keys?locale=${r.locale}&meaning=different` : `/app/${p.slug}/keys?locale=${r.locale}${d === "errors" ? "&issues=1" : "&status=missing"}`}
                              className="flex h-full items-center justify-center rounded-md font-mono text-sm tabular-nums transition-[filter] hover:brightness-110 focus-visible:ring-3 focus-visible:ring-ring/50"
                              style={{ background: ramp(c.score) }}
                            >
                              {pct(c.score)}
                            </Link>
                          </TooltipTrigger>
                          <TooltipContent>{t(`detail.${d}`, c.detail)}</TooltipContent>
                        </Tooltip>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
