"use client";

import { useQueries } from "@tanstack/react-query";
import { Hourglass, TriangleAlert } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { api, type XpSegmentRule } from "@/lib/api";

/**
 * How long until a result, per audience, from the project's own traffic and goal rate: days to detect +10 / +20 / +50 %,
 * and a plain warning with what to change when +20 % takes longer than 8 weeks.
 */
export function DurationEstimate({
  slug,
  segments,
  arms,
  goal,
  minN,
  trafficPct,
}: {
  slug: string;
  /** Targeted segments; "all traffic" is always estimated too. */
  segments: { name: string; rule: XpSegmentRule }[];
  /** Arms including the official text. */
  arms: number;
  goal: string;
  minN: number;
  trafficPct: number;
}) {
  const t = useTranslations("xpLifecycle.duration");
  const format = useFormatter();
  const rows = [{ name: t("allTraffic"), rule: null as XpSegmentRule | null }, ...segments];
  const qs = useQueries({
    queries: rows.map((r) => ({
      queryKey: ["xp-duration", slug, r.rule, arms, goal, minN, trafficPct],
      queryFn: () =>
        api.xpEstimate(slug, { rule: r.rule, arms: Math.max(2, arms), goal: goal || undefined, min_n: minN, traffic_pct: trafficPct }),
      retry: false,
      staleTime: 60_000,
    })),
  });
  const pct = (x: number) => format.number(x, { style: "percent", maximumFractionDigits: x < 0.01 ? 2 : 1 });
  const days = (d: number | null | undefined) =>
    d == null ? "—" : d > 730 ? t("overYears", { n: Math.floor(d / 365) }) : d > 60 ? t("weeks", { n: Math.ceil(d / 7) }) : t("days", { n: Math.ceil(d) });
  const data = qs.map((q) => q.data);
  const loading = qs.some((q) => q.isPending);
  const failed = qs.every((q) => q.isError);
  const noData = !loading && data.every((d) => !d || !d.baseline);
  // Slowest audience for +20 % drives the warning: every cell must reach its own sample size.
  const slow = data
    .map((d, i) => ({ d, i }))
    .filter(({ d }) => d?.baseline && (d.lifts[1]?.days == null || d.lifts[1].days > d.warn_days))
    .sort((a, b) => (a.d!.detectable_lift ?? Infinity) - (b.d!.detectable_lift ?? Infinity))
    .at(-1);
  const warnWeeks = Math.round((data.find(Boolean)?.warn_days ?? 56) / 7);

  return (
    <section aria-labelledby="xp-duration" className="space-y-3 rounded-lg border p-3 sm:p-4">
      <div className="flex items-start gap-2">
        <Hourglass className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
        <div className="min-w-0">
          <h3 id="xp-duration" className="text-sm font-medium">
            {t("title")}
          </h3>
          <p className="mt-0.5 text-xs text-pretty text-muted-foreground">{t("hint", { goal: goal || "signup", arms: Math.max(2, arms) })}</p>
        </div>
      </div>
      {loading ? (
        <div className="h-20 animate-pulse rounded bg-muted motion-reduce:animate-none" />
      ) : failed ? (
        <p className="text-sm text-muted-foreground">{t("unavailable")}</p>
      ) : noData ? (
        <p className="text-sm text-pretty text-muted-foreground">{t("noData", { goal: goal || "signup" })}</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full min-w-[28rem] text-sm">
            <thead>
              <tr className="text-start text-xs text-muted-foreground">
                <th scope="col" className="py-1 pe-3 text-start font-normal">{t("audience")}</th>
                <th scope="col" className="py-1 pe-3 text-end font-normal">{t("perDay")}</th>
                <th scope="col" className="py-1 pe-3 text-end font-normal">{t("rate")}</th>
                {[0.1, 0.2, 0.5].map((l) => (
                  <th key={l} scope="col" className="py-1 ps-2 text-end font-normal">
                    {t("lift", { lift: pct(l) })}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="tabular-nums">
              {rows.map((r, i) => {
                const d = data[i];
                return (
                  <tr key={i} className="border-t">
                    <th scope="row" className="max-w-40 truncate py-1.5 pe-3 text-start font-medium">
                      {r.name}
                      {d && !d.exact && <span className="ms-1 text-xs font-normal text-muted-foreground">{t("upperBound")}</span>}
                    </th>
                    <td className="py-1.5 pe-3 text-end">{d ? format.number(Math.round(d.daily_visitors)) : "—"}</td>
                    <td className="py-1.5 pe-3 text-end">{d?.baseline != null ? pct(d.baseline) : "—"}</td>
                    {(d?.lifts ?? [null, null, null]).map((l, j) => (
                      <td key={j} className={`py-1.5 ps-2 text-end ${l?.days != null && d && l.days <= d.warn_days ? "" : "text-muted-foreground"}`}>
                        {days(l?.days)}
                      </td>
                    ))}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {slow?.d && (
        <div role="alert" className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning-foreground" aria-hidden />
          <div className="min-w-0">
            <p className="font-medium text-pretty">
              {slow.d.detectable_lift != null
                ? t("tooSlow", { lift: pct(slow.d.detectable_lift), weeks: warnWeeks, audience: rows[slow.i].name })
                : t("tooSlowNone", { weeks: warnWeeks, audience: rows[slow.i].name })}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">{t("whatToDo")}</p>
            <ul className="mt-1 list-disc space-y-0.5 ps-4 text-xs">
              {arms > 2 && <li>{t("fewerVariants")}</li>}
              {segments.length > 0 && <li>{t("oneSegment")}</li>}
              <li>{t("frequentGoal")}</li>
              <li>{t("busierPage")}</li>
            </ul>
          </div>
        </div>
      )}
    </section>
  );
}
