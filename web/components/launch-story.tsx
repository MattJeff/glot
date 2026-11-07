"use client";

import { AlertTriangle, Coins, Lightbulb, ShieldCheck, Target, Users } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import type { LaunchStory, LaunchVerdict } from "@/lib/api";
import { cn } from "@/lib/utils";

// Positive wording only where the measurement supports it (see crates/server/src/launches.rs).
const TONE: Record<LaunchVerdict, string> = {
  collecting: "bg-muted text-muted-foreground",
  inconclusive: "bg-info/15 text-info-foreground",
  no_clear_change: "bg-info/15 text-info-foreground",
  probably_positive: "bg-success/15 text-success-foreground",
  higher_after: "bg-success/15 text-success-foreground",
  probably_negative: "bg-destructive/15 text-destructive",
  lower_after: "bg-destructive/15 text-destructive",
  invalid: "bg-warning/15 text-warning-foreground",
};

export function VerdictBadge({ verdict }: { verdict: LaunchVerdict }) {
  const t = useTranslations("launches.verdict");
  return (
    <Badge variant="secondary" className={cn("border-0 font-medium", TONE[verdict])}>
      {t(verdict)}
    </Badge>
  );
}

export function useSigned() {
  const format = useFormatter();
  return (n: number) => `${n >= 0 ? "+" : "−"}${format.number(Math.abs(n), { style: "percent", maximumFractionDigits: 1 })}`;
}

const rate = (c: { visits: number; conversions: number }) => (c.visits > 0 ? c.conversions / c.visits : 0);

/** Recommendation → cost → audience → result, then the curve. Aggregates only: safe on the public page. */
export function LaunchStoryView({ story: s }: { story: LaunchStory }) {
  const t = useTranslations("launches");
  const format = useFormatter();
  const name = useLocaleName();
  const signed = useSigned();
  const pct = (n: number) => format.number(n, { style: "percent", maximumFractionDigits: 1 });
  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD", maximumFractionDigits: n < 10 ? 2 : 0 });
  const date = (d: string) => format.dateTime(new Date(d), { day: "numeric", month: "short", year: "numeric" });
  const language = name(s.locale);
  const holdout = s.mode === "holdout";
  const r = s.recommendation;
  const labels = holdout
    ? [t("story.armFallback", { language: name(s.fallback) }), t("story.armNew", { language })]
    : [t("story.armBefore"), t("story.armAfter")];
  const span = Math.max(Math.abs(s.lift_lo), Math.abs(s.lift_hi), 0.05) * 1.1;
  const pos = (n: number) => `${50 + (Math.max(-span, Math.min(span, n)) / span) * 50}%`;
  const excluded = s.explicit[0] + s.explicit[1];

  const steps = [
    {
      icon: Lightbulb,
      title: t("story.step1"),
      body:
        r?.rank != null && r.of != null
          ? t("story.reco", {
              language,
              rank: r.rank,
              of: r.of,
              share: pct(r.share ?? 0),
              cost: usd(r.cost_usd ?? 0),
              confidence: r.confidence ?? "insufficient",
            })
          : t("story.recoNone"),
    },
    { icon: Coins, title: t("story.step2"), body: t(s.cost_estimated ? "story.costEstimated" : "story.cost", { cost: usd(s.cost_usd) }) },
    {
      icon: Users,
      title: t("story.step3"),
      body: t("story.audience", { visits: format.number(s.baseline.visits), language, days: s.days, rate: pct(rate(s.baseline)), goal: s.goal }),
    },
  ];

  return (
    <article className="space-y-6">
      <header>
        <div className="mb-2 text-xs font-medium text-muted-foreground">{t("story.kicker")}</div>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <h1 className="text-2xl font-semibold tracking-tight text-balance sm:text-3xl">{t("story.headline", { project: s.project, language })}</h1>
          <VerdictBadge verdict={s.verdict} />
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          {t("story.period", {
            from: date(s.launched_at),
            to: date(s.decided_at ?? s.ends_at),
            mode: t(`mode.${s.mode}`),
          })}
        </p>
      </header>

      <ol className="grid gap-3 md:grid-cols-3">
        {steps.map((st, i) => (
          <li key={st.title} className="rounded-xl border p-4">
            <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
              <span className="grid size-5 place-items-center rounded-full bg-muted text-[11px] tabular-nums">{i + 1}</span>
              <st.icon className="size-3.5" /> {st.title}
            </div>
            <p className="mt-2 text-sm">{st.body}</p>
          </li>
        ))}
      </ol>

      <section className="rounded-xl border p-4 sm:p-5">
        <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <span className="grid size-5 place-items-center rounded-full bg-muted text-[11px] tabular-nums">4</span>
          <Target className="size-3.5" /> {t("story.step4")}
        </div>
        <div className="mt-3 grid grid-cols-2 gap-3 text-sm">
          {s.arms.map((a, i) => (
            <div key={i} className="rounded-md bg-muted/40 p-3">
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span className={cn("size-2.5 rounded-sm", i ? "bg-info" : "bg-muted-foreground/50")} /> {labels[i]}
              </div>
              <div className="mt-1 text-lg font-semibold tabular-nums">{pct(rate(a))}</div>
              <div className="text-xs text-muted-foreground tabular-nums">
                {format.number(a.conversions)} / {format.number(a.visits)}
              </div>
            </div>
          ))}
        </div>

        <div className="mt-4">
          <div className="flex flex-wrap items-baseline justify-between gap-2 text-xs">
            <span className="text-muted-foreground">{t("story.lift", { goal: s.goal })}</span>
            <span className="tabular-nums">
              <b className="text-base font-semibold">{signed(s.lift)}</b>{" "}
              <span className="text-muted-foreground">
                {t("story.interval")} [{signed(s.lift_lo)} ; {signed(s.lift_hi)}]
              </span>
            </span>
          </div>
          <div className="relative mt-2 h-4" role="img" aria-label={`${t("story.interval")}: ${signed(s.lift_lo)} – ${signed(s.lift_hi)}`}>
            <div className="absolute inset-x-0 top-1/2 h-px bg-border" />
            <div className="absolute inset-y-0 start-1/2 w-px bg-foreground/40" />
            <div
              className={cn("absolute top-1/2 h-1 -translate-y-1/2 rounded-full", s.lift_lo > 0 ? "bg-success/60" : s.lift_hi < 0 ? "bg-destructive/50" : "bg-info/45")}
              style={{ insetInlineStart: pos(s.lift_lo), width: `calc(${pos(s.lift_hi)} - ${pos(s.lift_lo)})` }}
            />
            <div
              className="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground ring-2 ring-background rtl:translate-x-1/2"
              style={{ insetInlineStart: pos(s.lift) }}
            />
          </div>
          {holdout && <p className="mt-2 text-xs text-muted-foreground">{t("story.pBetter", { language, p: pct(s.p_better) })}</p>}
          <p className="mt-2 text-sm">{t(`explain.${s.verdict}`)}</p>
        </div>

        {!s.causal && (
          <p className="mt-3 flex gap-2 rounded-md bg-warning/10 p-3 text-xs">
            <AlertTriangle className="size-4 shrink-0 text-warning-foreground" /> {t("story.caveat")}
          </p>
        )}
        {s.verdict === "invalid" && (
          <p className="mt-3 flex gap-2 rounded-md bg-warning/10 p-3 text-xs">
            <AlertTriangle className="size-4 shrink-0 text-warning-foreground" /> {t("story.srm", { pct: s.holdout_pct })}
          </p>
        )}
        {holdout && <p className="mt-3 text-xs text-muted-foreground">{t("story.scope")}</p>}
        {excluded > 0 && <p className="mt-1 text-xs text-muted-foreground">{t("story.explicit", { n: excluded })}</p>}
      </section>

      {s.guardrails.length > 0 && (
        <section className="rounded-xl border p-4 sm:p-5">
          <h2 className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <ShieldCheck className="size-3.5" /> {t("story.guardrails")}
          </h2>
          <p className="mt-1 text-xs text-muted-foreground">{t("story.guardrailsHint")}</p>
          <ul className="mt-3 divide-y text-sm">
            {s.guardrails.map((g) => (
              <li key={g.event} className="flex flex-wrap items-center justify-between gap-2 py-2">
                <span className="min-w-0">
                  <span className="font-mono text-xs" translate="no">
                    {g.event}
                  </span>{" "}
                  <span className="text-xs text-muted-foreground">· {t(g.higher_is_worse ? "story.guardrailUp" : "story.guardrailDown")}</span>
                  <span className="block text-xs text-muted-foreground tabular-nums">
                    {labels[0]} {pct(s.arms[0].visits > 0 ? g.counts[0] / s.arms[0].visits : 0)} · {labels[1]}{" "}
                    {pct(s.arms[1].visits > 0 ? g.counts[1] / s.arms[1].visits : 0)} · {t("story.pWorse", { p: pct(g.p_worse) })}
                  </span>
                </span>
                <Badge
                  variant="secondary"
                  className={cn(
                    "border-0 font-medium",
                    g.status === "ok" ? "bg-success/15 text-success-foreground" : g.status === "degraded" ? "bg-destructive/15 text-destructive" : "bg-muted text-muted-foreground",
                  )}
                >
                  {t(`guardrailStatus.${g.status}`)}
                </Badge>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="rounded-xl border p-4 sm:p-5">
        <h2 className="mb-3 text-sm font-medium">{t("story.chart")}</h2>
        <CumulativeChart story={s} labels={labels} />
      </section>

      <p className="text-xs text-muted-foreground">{t("story.footer")}</p>
    </article>
  );
}

/** Cumulative conversion rate per series and day (each series starts on its first visit). */
function CumulativeChart({ story: s, labels }: { story: LaunchStory; labels: string[] }) {
  const t = useTranslations("launches.story");
  const format = useFormatter();
  const days = s.daily;
  if (days.length < 2) return <p className="text-sm text-muted-foreground">—</p>;
  const series = [0, 1].map((i) => {
    let v = 0,
      c = 0;
    return days.map((d) => {
      v += d.visits[i];
      c += d.conversions[i];
      return v > 0 ? c / v : null;
    });
  });
  const max = Math.max(0.001, ...series.flat().filter((x): x is number => x != null)) * 1.15;
  const x = (i: number) => (i / (days.length - 1)) * 600;
  const y = (r: number) => 160 - (r / max) * 150;
  const path = (pts: (number | null)[]) =>
    pts
      .map((r, i) => (r == null ? null : `${x(i).toFixed(1)},${y(r).toFixed(1)}`))
      .filter(Boolean)
      .join(" ");
  const day = (d: string) => format.dateTime(new Date(`${d}T00:00:00Z`), { day: "numeric", month: "short", timeZone: "UTC" });
  const launch = days.findIndex((d) => d.day >= s.launched_at.slice(0, 10));

  return (
    <figure>
      <div className="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {labels.map((l, i) => (
          <span key={l} className="flex items-center gap-1.5">
            <span className={cn("size-2.5 rounded-sm", i ? "bg-info" : "bg-muted-foreground/50")} /> {l}
          </span>
        ))}
        <span className="ms-auto tabular-nums">{format.number(max, { style: "percent", maximumFractionDigits: 1 })}</span>
      </div>
      <svg
        viewBox="0 0 600 160"
        preserveAspectRatio="none"
        className="h-40 w-full border-b"
        role="img"
        aria-label={t("chartAria", { goal: s.goal, a: labels[0], b: labels[1] })}
      >
        {!s.causal && launch > 0 && <line x1={x(launch)} x2={x(launch)} y1={0} y2={160} className="stroke-border" strokeDasharray="4 4" vectorEffect="non-scaling-stroke" />}
        <polyline points={path(series[0])} fill="none" className="stroke-muted-foreground/60" strokeWidth={2} vectorEffect="non-scaling-stroke" />
        <polyline points={path(series[1])} fill="none" className="stroke-info" strokeWidth={2} vectorEffect="non-scaling-stroke" />
      </svg>
      <div className="mt-1.5 flex justify-between text-[11px] text-muted-foreground tabular-nums">
        <span>{day(days[0].day)}</span>
        <span>{day(days.at(-1)!.day)}</span>
      </div>
    </figure>
  );
}
