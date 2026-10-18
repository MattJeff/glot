"use client";

import { CirclePause, CirclePlay, Clock, Eye, Loader2, Square, Target, Trophy, Users } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { useCelebrationsEnabled } from "@/components/gamification/project-progress";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { XpExperiment, XpStatus } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { Act } from "./cell-card";
import { Confirm } from "./confirm";
import { type CellView, useArmLabel, useXpFormat } from "./shared";

const STATUS_STYLE: Record<XpStatus, string> = {
  draft: "bg-muted text-muted-foreground",
  running: "bg-info/12 text-info-foreground",
  paused: "bg-warning/15 text-warning-foreground",
  personalized: "bg-brand/12 text-foreground",
  promoted: "bg-muted text-muted-foreground",
  stopped: "bg-muted text-muted-foreground",
};

export function XpStatusBadge({ status }: { status: XpStatus }) {
  const t = useTranslations("experimentResults.status");
  return (
    <Badge variant="secondary" className={cn("border-0 font-medium", STATUS_STYLE[status])}>
      {t(status)}
    </Badge>
  );
}

/** The whole experiment in one or two sentences, for someone who never took a stats class. */
export function useVerdict(exp: XpExperiment, cells: CellView[]) {
  const t = useTranslations("experimentResults");
  const { signed, prob } = useXpFormat();
  const label = useArmLabel(exp);
  const only = cells.length === 1;
  const seg = (c: CellView) => ({ star: c.id === "*" ? (only ? "only" : "rest") : "named", segment: c.name });
  if (exp.status === "draft") return t("verdict.draft");
  if (exp.status === "promoted") return t("verdict.promoted", { arm: label(exp.state.find((s) => s.winner != null)?.winner ?? 0) });
  const parts: string[] = [];
  let pending = 0;
  for (const c of cells) {
    const w = c.arms.find((a) => a.arm === c.winner);
    if (c.state === "personalized" && c.winner != null) parts.push(t("verdict.personalized", { arm: label(c.winner), ...seg(c) }));
    else if (c.state === "winner" && c.winner === 0) parts.push(t("verdict.officialWins", seg(c)));
    else if (c.state === "winner" && c.winner != null)
      parts.push(
        w
          ? t(exp.metric === "revenue" ? "verdict.winRevenue" : "verdict.win", { arm: label(c.winner), lift: signed(w.lift), p: prob(w.p_best), ...seg(c) })
          : t("verdict.winShort", { arm: label(c.winner), ...seg(c) }),
      );
    else if (c.state === "learning") pending++;
  }
  if (!parts.length) {
    if (exp.status === "stopped") return t("verdict.stopped");
    if (exp.started_at && cells.every((c) => !c.views) && cells.some((c) => c.arms.length)) return t("verdict.noTraffic");
    if (pending) {
      const days = Math.max(-1, ...cells.map((c) => (c.days_left == null ? -1 : Math.max(1, Math.round(c.days_left)))));
      return t("verdict.learning", { days });
    }
    return t("verdict.noDifference");
  }
  if (pending) parts.push(t("verdict.elsewhere"));
  return parts.join(" ");
}

/**
 * Celebrates a segment's first winner once (toast + a small pop on the trophy), and the project's
 * very first winner as a milestone. Opt-out shared with the rest of the app; nothing moves with reduced motion.
 */
export function useWinnerCelebration(exp: XpExperiment, cells: CellView[], slug: string) {
  const t = useTranslations("experimentResults");
  const [enabled] = useCelebrationsEnabled();
  const trophy = useRef<HTMLSpanElement>(null);
  const won = cells
    .filter((c) => c.state === "winner" && c.winner)
    .map((c) => c.id)
    .join(",");
  useEffect(() => {
    const winners = won ? won.split(",") : [];
    const key = `glot:xp-winners:${exp.id}`;
    const projectKey = `glot:xp-first-winner:${slug}`;
    let seen: string[] | null = null;
    let first = false;
    try {
      seen = JSON.parse(localStorage.getItem(key) ?? "null");
      first = !localStorage.getItem(projectKey);
      localStorage.setItem(key, JSON.stringify(winners));
      if (winners.length) localStorage.setItem(projectKey, "1");
    } catch {
      /* private mode: no celebration memory, stay quiet */
      return;
    }
    // First visit records silently unless it is the project's first winner ever.
    const news = seen ? winners.filter((w) => !seen.includes(w)) : first ? winners : [];
    if (!enabled || !news.length) return;
    // Small pop on the verdict trophy; skipped with reduced motion.
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches)
      trophy.current?.animate([{ transform: "scale(0.6)" }, { transform: "scale(1.15)" }, { transform: "scale(1)" }], { duration: 320, easing: "cubic-bezier(0.22, 1, 0.36, 1)" });
    toast.success(first ? t("celebrate.first") : t("celebrate.winner"), { description: t("celebrate.body") });
  }, [exp.id, won, enabled, slug, t]);
  return trophy;
}

export function SummaryHeader({
  exp,
  cells,
  slug,
  busy,
  onAct,
}: {
  exp: XpExperiment;
  cells: CellView[];
  slug: string;
  busy: boolean;
  onAct: (a: Act) => void;
}) {
  const t = useTranslations("experimentResults");
  const { format } = useXpFormat();
  const verdict = useVerdict(exp, cells);
  const trophy = useWinnerCelebration(exp, cells, slug);
  const hasWinner = cells.some((c) => c.state === "winner" && c.winner);
  const start = exp.started_at ? new Date(exp.started_at) : null;
  const end = exp.ended_at ? new Date(exp.ended_at) : new Date();
  const days = start ? Math.max(0, Math.floor((end.getTime() - start.getTime()) / 86_400_000)) : null;
  const live = exp.status === "running" || exp.status === "paused" || exp.status === "personalized";

  const stats = [
    { icon: Clock, label: t("summary.duration"), value: days == null ? "—" : t("summary.days", { n: days }), hint: start ? t("summary.since", { date: format.dateTime(start, { dateStyle: "medium" }) }) : null },
    { icon: Users, label: t("summary.traffic"), value: format.number(cells.reduce((n, c) => n + c.views, 0)), hint: t("summary.segments", { n: cells.length }) },
    { icon: Target, label: t("summary.goal"), value: exp.goal, hint: exp.metric === "revenue" ? t("summary.revenue") : t(`summary.allocation.${exp.allocation}`), mono: true },
  ];

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0 max-w-3xl">
          <div className="mb-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <Link href={`/app/${slug}/experiments`} className="rounded-sm hover:text-foreground hover:underline">
              {t("back")}
            </Link>
            <span aria-hidden>/</span>
            <XpStatusBadge status={exp.status} />
          </div>
          <h1 className="text-2xl font-semibold tracking-tight text-balance">{exp.name}</h1>
          <p className="mt-1 flex flex-wrap gap-x-2 text-xs text-muted-foreground">
            {exp.keys.map((k) => (
              <code key={k.key} dir="ltr" translate="no" className="font-mono">
                {k.key}
              </code>
            ))}
            <span>· {t("summary.locales", { n: exp.locales.length })}</span>
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" asChild>
            <Link href={`/app/${slug}/preview?xp=${exp.id}`}>
              <Eye aria-hidden /> {t("previewInPage")}
            </Link>
          </Button>
          {exp.status === "running" && (
            <Button variant="outline" size="sm" disabled={busy} onClick={() => onAct({ action: "pause" })}>
              {busy ? <Loader2 className="animate-spin" aria-hidden /> : <CirclePause aria-hidden />} {t("pause")}
            </Button>
          )}
          {exp.status === "paused" && (
            <Button size="sm" disabled={busy} onClick={() => onAct({ action: "resume" })}>
              {busy ? <Loader2 className="animate-spin" aria-hidden /> : <CirclePlay aria-hidden />} {t("resume")}
            </Button>
          )}
          {live && (
            <Confirm
              trigger={
                <Button variant="ghost" size="sm" disabled={busy}>
                  <Square aria-hidden /> {t("stop")}
                </Button>
              }
              title={t("confirmStop.title", { name: exp.name })}
              body={<p>{t("confirmStop.body")}</p>}
              confirm={t("confirmStop.cta")}
              onConfirm={() => onAct({ action: "stop" })}
            />
          )}
        </div>
      </div>

      <div
        className={cn(
          "flex gap-3 rounded-xl border p-4 sm:p-5",
          hasWinner ? "border-success/40 bg-success/5" : "bg-muted/30",
        )}
      >
        <span
          ref={trophy}
          aria-hidden
          className={cn(
            "grid size-9 shrink-0 place-items-center rounded-full",
            hasWinner ? "bg-success/15 text-success-foreground" : "bg-muted text-muted-foreground",
          )}
        >
          <Trophy className="size-4" />
        </span>
        <div className="min-w-0">
          <h2 className="text-xs font-medium text-muted-foreground">{t("summary.verdict")}</h2>
          <p role="status" aria-live="polite" className="mt-0.5 text-base text-pretty">
            {verdict}
          </p>
          {exp.status === "paused" && <p className="mt-1 text-sm text-muted-foreground">{t("summary.pausedNote")}</p>}
        </div>
      </div>

      <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {stats.map((s) => (
          <div key={s.label} className="rounded-xl border p-4">
            <dt className="flex items-center gap-2 text-xs text-muted-foreground">
              <s.icon className="size-3.5" aria-hidden /> {s.label}
            </dt>
            <dd className={cn("mt-1.5 truncate text-xl font-semibold tracking-tight tabular-nums", s.mono && "font-mono text-base")}>{s.value}</dd>
            {s.hint && <dd className="mt-1 truncate text-xs text-muted-foreground">{s.hint}</dd>}
          </div>
        ))}
      </dl>
    </div>
  );
}
