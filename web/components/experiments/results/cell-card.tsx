"use client";

import { useTranslations } from "next-intl";

import { Languages, Loader2, Pin, ShieldAlert, Trophy, TrendingDown, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { XpAction, XpExperiment } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Confirm } from "./confirm";
import { LiftChart } from "./lift-chart";
import { ArmSwatch, type CellView, CellStateBadge, SRM_P, useArmLabel, useXpFormat } from "./shared";

export type Act = XpAction | { action: "propagate"; arm: number };

/** One segment ("cell"): a sentence, the lift chart, the numbers, its warnings and its promotion. */
export function CellCard({
  exp,
  cell,
  only,
  otherLocales,
  busy,
  onAct,
}: {
  exp: XpExperiment;
  cell: CellView;
  only: boolean;
  /** Project languages this experiment doesn't cover yet (XP9 "test in other languages"). */
  otherLocales: number;
  busy: boolean;
  onAct: (a: Act) => void;
}) {
  const t = useTranslations("experimentResults");
  const { format, pct, prob, signed, money, cellName } = useXpFormat();
  const label = useArmLabel(exp);
  const name = cellName(cell, only);
  const views = cell.views;
  const revenue = exp.metric === "revenue";
  const best = [...cell.arms].sort((a, b) => b.p_best - a.p_best)[0];
  const winner = cell.arms.find((a) => a.arm === cell.winner);
  const srm = cell.srm_p != null && cell.srm_p < SRM_P;
  const live = exp.status === "running" || exp.status === "paused" || exp.status === "personalized";
  const canPromote = live && cell.state === "winner" && winner && winner.arm !== 0 && !srm;

  // The one-sentence answer for this segment (ux: numbers + plain words, ranges instead of jargon).
  let sentence: string;
  if (cell.state === "personalized" && cell.winner != null) sentence = t("cell.personalized", { arm: label(cell.winner) });
  else if (views === 0 || !best) sentence = t("cell.noTraffic");
  else if (cell.state === "winner" && winner) {
    sentence =
      winner.arm === 0
        ? t("cell.officialWins")
        : t(revenue ? "cell.winnerRevenue" : "cell.winner", {
            arm: label(winner.arm),
            lift: signed(winner.lift),
            lo: signed(winner.lift_lo),
            hi: signed(winner.lift_hi),
            goal: exp.goal,
            p: prob(winner.p_best),
          });
  } else if (cell.state === "no_difference") sentence = t("cell.noDifference");
  else
    sentence = t("cell.learning", {
      arm: label(best.arm),
      p: prob(best.p_best),
      days: cell.days_left == null ? -1 : Math.max(1, Math.round(cell.days_left)),
    });

  const headingId = `cell-${cell.id}`;
  return (
    <section aria-labelledby={headingId} className="rounded-xl border bg-card p-4 shadow-(--shadow-card) sm:p-5">
      <header className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
        <div className="min-w-0">
          <h3 id={headingId} className="flex items-center gap-2 font-medium">
            <Users className="size-4 text-muted-foreground" aria-hidden /> {name}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground tabular-nums">{t("cell.visitors", { n: views })}</p>
        </div>
        <CellStateBadge state={cell.state} />
      </header>

      <p className="mt-3 text-sm text-pretty">{sentence}</p>

      {srm && (
        <div className="mt-3 flex gap-2.5 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm">
          <ShieldAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
          <div>
            <p className="font-medium">{t("srm.title")}</p>
            <p className="mt-0.5 text-muted-foreground">{t("srm.body", { p: format.number(cell.srm_p!, { maximumSignificantDigits: 2 }) })}</p>
          </div>
        </div>
      )}
      {cell.underpowered && !srm && (
        <div className="mt-3 flex gap-2.5 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
          <TrendingDown className="mt-0.5 size-4 shrink-0 text-warning-foreground" aria-hidden />
          <div>
            <p className="font-medium">{t("underpowered.title")}</p>
            <p className="mt-0.5 text-muted-foreground">{t("underpowered.body")}</p>
          </div>
        </div>
      )}

      {views > 0 && cell.arms.length > 0 && (
        <>
          <div className="mt-4">
            <LiftChart exp={exp} cell={cell} />
          </div>
          <div className="mt-4 -mx-4 overflow-x-auto sm:-mx-5">
            <table className="w-full min-w-[34rem] text-sm">
              <caption className="sr-only">{t("table.caption", { segment: name })}</caption>
              <thead className="border-y bg-muted/40 text-xs text-muted-foreground">
                <tr>
                  <th scope="col" className="px-4 py-2 text-start font-medium sm:ps-5">{t("table.arm")}</th>
                  <th scope="col" className="px-3 py-2 text-end font-medium">{t("table.visitors")}</th>
                  <th scope="col" className="px-3 py-2 text-end font-medium">{revenue ? t("table.rpv") : t("table.rate", { goal: exp.goal })}</th>
                  <th scope="col" className="px-3 py-2 text-end font-medium">{t("table.lift")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium sm:pe-5">{t("table.pBest")}</th>
                </tr>
              </thead>
              <tbody className="divide-y">
                {cell.arms.map((a) => {
                  const won = cell.winner === a.arm && cell.state === "winner";
                  return (
                    <tr key={a.arm} className={cn(won && "bg-success/5")}>
                      <th scope="row" className="px-4 py-2.5 text-start font-normal sm:ps-5">
                        <span className="flex items-center gap-2">
                          <ArmSwatch arm={a.arm} />
                          <span className="font-medium">{label(a.arm)}</span>
                          {won && (
                            <span className="inline-flex items-center gap-1 text-xs text-success-foreground">
                              <Trophy className="size-3" aria-hidden /> {t("winner")}
                            </span>
                          )}
                        </span>
                      </th>
                      <td className="px-3 py-2.5 text-end tabular-nums">{a.views == null ? "—" : format.number(a.views)}</td>
                      <td className="px-3 py-2.5 text-end tabular-nums">
                        {revenue ? money(a.mean, exp.currency) : pct(a.mean, 1)}
                        {a.views != null && a.conversions != null && (
                          <div className="text-xs text-muted-foreground">
                            {format.number(a.conversions)} / {format.number(a.views)}
                          </div>
                        )}
                      </td>
                      <td className="px-3 py-2.5 text-end tabular-nums">
                        {a.arm === 0 ? (
                          <span className="text-muted-foreground">{t("reference")}</span>
                        ) : (
                          <>
                            {signed(a.lift)}
                            <div className="text-xs text-muted-foreground">{t("table.range", { lo: signed(a.lift_lo), hi: signed(a.lift_hi) })}</div>
                          </>
                        )}
                      </td>
                      <td className="px-4 py-2.5 sm:pe-5">
                        <div className="flex items-center justify-end gap-2">
                          <div className="hidden h-1.5 w-16 overflow-hidden rounded-full bg-muted sm:block" aria-hidden>
                            <div
                              className={cn("h-full rounded-full", a.p_best > 0.95 ? "bg-success" : "bg-foreground/50")}
                              style={{ width: `${Math.max(2, a.p_best * 100)}%` }}
                            />
                          </div>
                          <span className="w-12 text-end tabular-nums">{prob(a.p_best)}</span>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </>
      )}

      {canPromote && winner && (
        <footer className="mt-4 flex flex-wrap items-center gap-2 border-t pt-4">
          <Confirm
            trigger={
              <Button size="sm" disabled={busy}>
                {busy ? <Loader2 className="animate-spin" aria-hidden /> : <Pin aria-hidden />} {t("promoteSegment")}
              </Button>
            }
            title={t("confirmSegment.title", { arm: label(winner.arm), segment: name })}
            body={
              <>
                <p>{t("confirmSegment.body", { segment: name, locales: exp.locales.length })}</p>
                <p>{t("confirmSegment.after")}</p>
              </>
            }
            confirm={t("confirmSegment.cta")}
            onConfirm={() => onAct({ action: "promote", scope: "segment", cell: cell.id, arm: winner.arm })}
          />
          <Confirm
            trigger={
              <Button size="sm" variant="outline" disabled={busy}>
                <Trophy aria-hidden /> {t("promoteAll")}
              </Button>
            }
            title={t("confirmAll.title", { arm: label(winner.arm) })}
            body={
              <>
                <p>{t("confirmAll.body", { keys: exp.keys.length, locales: exp.locales.length })}</p>
                <p>{t("confirmAll.after")}</p>
              </>
            }
            confirm={t("confirmAll.cta")}
            onConfirm={() => onAct({ action: "promote", scope: "all", arm: winner.arm })}
          />
          {otherLocales > 0 && (
            <Confirm
              trigger={
                <Button size="sm" variant="ghost" disabled={busy}>
                  <Languages aria-hidden /> {t("propagate")}
                </Button>
              }
              title={t("confirmPropagate.title", { arm: label(winner.arm) })}
              body={<p>{t("confirmPropagate.body", { segment: name, n: otherLocales })}</p>}
              confirm={t("confirmPropagate.cta")}
              onConfirm={() => onAct({ action: "propagate", arm: winner.arm })}
            />
          )}
        </footer>
      )}
    </section>
  );
}
