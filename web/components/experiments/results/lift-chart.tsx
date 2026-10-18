"use client";

import { useTranslations } from "next-intl";

import type { XpExperiment } from "@/lib/api";
import { cn } from "@/lib/utils";
import { ArmSwatch, type CellView, seriesOf, useArmLabel, useXpFormat } from "./shared";

/**
 * Forest plot: one row per variant, dot = most likely lift vs official, whisker = 95 % credible interval,
 * centre line = official. Plain HTML positioned in %, so it reflows at any width (and mirrors in RTL).
 * The ArmsTable below it is the legend and the accessible data view.
 */
export function LiftChart({ exp, cell }: { exp: XpExperiment; cell: CellView }) {
  const t = useTranslations("experimentResults");
  const { signed } = useXpFormat();
  const label = useArmLabel(exp);
  const rows = cell.arms.filter((a) => a.arm !== 0).map((a) => ({ a, ci: [a.lift_lo, a.lift_hi] }));
  if (!rows.length) return null;
  const D = Math.max(0.1, ...rows.flatMap((r) => r.ci.map(Math.abs))) * 1.15;
  const x = (v: number) => `${50 + (Math.max(-D, Math.min(D, v)) / D) * 50}%`;
  const ticks = [-D / 2, 0, D / 2];

  return (
    <figure className="space-y-1">
      <div
        role="img"
        aria-label={t("chart.aria", {
          summary: rows.map((r) => t("chart.row", { arm: label(r.a.arm), lift: signed(r.a.lift), lo: signed(r.ci[0]), hi: signed(r.ci[1]) })).join("; "),
        })}
        className="space-y-1"
      >
        {rows.map(({ a, ci }) => (
          <div key={a.arm} className="grid grid-cols-1 items-center gap-x-3 sm:grid-cols-[10rem_1fr]">
            <div className="flex min-w-0 items-center gap-2 text-sm">
              <ArmSwatch arm={a.arm} />
              <span className="truncate">{label(a.arm)}</span>
            </div>
            <div
              className={cn("relative h-8", seriesOf(a.arm))}
              title={t("chart.row", { arm: label(a.arm), lift: signed(a.lift), lo: signed(ci[0]), hi: signed(ci[1]) })}
            >
              <span aria-hidden className="absolute inset-y-0 start-1/2 border-s border-dashed border-muted-foreground/60" />
              <span
                aria-hidden
                className="absolute top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-current opacity-60"
                style={{ insetInlineStart: x(ci[0]), insetInlineEnd: `calc(100% - ${x(ci[1])})` }}
              />
              <span
                aria-hidden
                className="absolute top-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-current ring-2 ring-card rtl:translate-x-1/2"
                style={{ insetInlineStart: x(a.lift) }}
              />
              <span
                aria-hidden
                className={cn(
                  "absolute top-1/2 -translate-y-1/2 px-1.5 text-xs font-medium text-foreground tabular-nums",
                  a.lift >= 0 ? "" : "-translate-x-full rtl:translate-x-full",
                )}
                style={{ insetInlineStart: a.lift >= 0 ? x(Math.min(D, ci[1])) : x(Math.max(-D, ci[0])) }}
              >
                {signed(a.lift)}
              </span>
            </div>
          </div>
        ))}
      </div>
      <div aria-hidden className="grid grid-cols-1 gap-x-3 sm:grid-cols-[10rem_1fr]">
        <span className="hidden sm:block" />
        <div className="relative h-4 text-[11px] text-muted-foreground tabular-nums">
          {ticks.map((v) => (
            <span key={v} className="absolute -translate-x-1/2 rtl:translate-x-1/2" style={{ insetInlineStart: x(v) }}>
              {v === 0 ? t("official") : signed(v)}
            </span>
          ))}
        </div>
      </div>
      <figcaption className="text-xs text-muted-foreground">{t("chart.caption")}</figcaption>
    </figure>
  );
}
