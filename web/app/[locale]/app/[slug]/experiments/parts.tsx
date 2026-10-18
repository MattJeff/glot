"use client";

import { Check, CircleDashed, Loader2, Square, Trophy, X } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useId, useRef, useState } from "react";
import { useLocaleName } from "@/components/common";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { VariantArm, VariantExperiment } from "@/lib/api";
import { cn } from "@/lib/utils";

// Official = neutral reference; variants take categorical slots 1-3 (validated all-pairs, light + dark steps).
const SERIES = [
  "text-muted-foreground",
  "text-[#2a78d6] dark:text-[#3987e5]",
  "text-[#eb6834] dark:text-[#d95926]",
  "text-[#1baf7a] dark:text-[#199e70]",
];
const LETTERS = "ABC";
const TONES = ["direct", "warm", "urgent", "official"] as const;
type Tone = (typeof TONES)[number];

export function useArmName() {
  const t = useTranslations("experiments");
  return (a: VariantArm) => (a.is_control ? t("official") : t("variant", { letter: LETTERS[a.arm - 1] ?? a.arm }));
}

// ---------- Beta posterior maths (display only; decisions are made server-side) ----------

// Lanczos log-gamma (g = 7), plenty for plotting.
const LG = [0.99999999999980993, 676.5203681218851, -1259.1392167224028, 771.32342877765313, -176.61502916214059, 12.507343278686905, -0.13857109526572012, 9.9843695780195716e-6, 1.5056327351493116e-7];
function lgamma(x: number): number {
  if (x < 0.5) return Math.log(Math.PI / Math.sin(Math.PI * x)) - lgamma(1 - x);
  x -= 1;
  let a = LG[0];
  const t = x + 7.5;
  for (let i = 1; i < 9; i++) a += LG[i] / (x + i);
  return 0.5 * Math.log(2 * Math.PI) + (x + 0.5) * Math.log(t) - t + Math.log(a);
}
const lbeta = (a: number, b: number) => lgamma(a) + lgamma(b) - lgamma(a + b);
const pdf = (x: number, a: number, b: number) =>
  x <= 0 || x >= 1 ? 0 : Math.exp((a - 1) * Math.log(x) + (b - 1) * Math.log(1 - x) - lbeta(a, b));
export function moments(a: VariantArm) {
  const n = a.alpha + a.beta;
  const mean = a.alpha / n;
  const sd = Math.sqrt((a.alpha * a.beta) / (n * n * (n + 1)));
  return { mean, sd, lo: Math.max(0, mean - 1.96 * sd), hi: Math.min(1, mean + 1.96 * sd) };
}

/** Overlaid Beta posteriors of the conversion rate, one curve per arm. */
export function PosteriorChart({ arms }: { arms: VariantArm[] }) {
  const t = useTranslations("experiments");
  const format = useFormatter();
  const armName = useArmName();
  const clip = useId();
  const pct = (n: number) => format.number(n, { style: "percent", maximumFractionDigits: 1 });
  // Drawn at the real container width so labels keep their size on phones.
  const box = useRef<HTMLElement>(null);
  const [W, setW] = useState(720);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setW(Math.max(280, Math.round(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const H = W < 500 ? 150 : 190, L = 8, R = 8, T = 22, B = 26;
  const m = arms.map(moments);
  let lo = Math.max(0, Math.min(...m.map((x) => x.mean - 4 * x.sd)));
  let hi = Math.min(1, Math.max(...m.map((x) => x.mean + 4 * x.sd)));
  if (hi - lo < 0.002) [lo, hi] = [Math.max(0, lo - 0.001), Math.min(1, hi + 0.001)];
  const N = 160;
  const xs = Array.from({ length: N + 1 }, (_, i) => lo + ((hi - lo) * i) / N);
  const curves = arms.map((a) => xs.map((x) => pdf(x, a.alpha, a.beta)));
  const ymax = Math.max(...curves.flat(), 1e-9);
  const sx = (x: number) => L + ((x - lo) / (hi - lo)) * (W - L - R);
  const sy = (y: number) => H - B - (y / ymax) * (H - B - T);
  const nt = W < 500 ? 3 : 5;
  const ticks = Array.from({ length: nt }, (_, i) => lo + ((hi - lo) * i) / (nt - 1));
  // Direct labels at each peak, skipped when they would collide with one already placed.
  const placed: { x: number; y: number }[] = [];
  const labels = arms.map((a, i) => {
    const k = curves[i].indexOf(Math.max(...curves[i]));
    const p = { x: sx(xs[k]), y: Math.max(12, sy(curves[i][k]) - 6) };
    if (placed.some((q) => Math.abs(q.x - p.x) < 70 && Math.abs(q.y - p.y) < 14)) return null;
    placed.push(p);
    return p;
  });

  return (
    <figure ref={box}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="h-auto w-full overflow-visible"
        role="img"
        aria-label={t("chartAria", {
          summary: arms.map((a, i) => `${armName(a)} ${pct(m[i].mean)} [${pct(m[i].lo)}–${pct(m[i].hi)}]`).join(", "),
        })}
      >
        <clipPath id={clip}>
          <rect x={L} y={0} width={W - L - R} height={H - B} />
        </clipPath>
        <line x1={L} x2={W - R} y1={H - B} y2={H - B} className="stroke-border" />
        {ticks.map((x) => (
          <text key={x} x={Math.min(W - R - 14, Math.max(L + 14, sx(x)))} y={H - 8} textAnchor="middle" className="fill-muted-foreground text-[11px] tabular-nums">
            {pct(x)}
          </text>
        ))}
        <g clipPath={`url(#${clip})`}>
          {arms.map((a, i) => {
            const d = xs.map((x, j) => `${j ? "L" : "M"}${sx(x).toFixed(1)},${sy(curves[i][j]).toFixed(1)}`).join("");
            return (
              <g key={a.arm} className={SERIES[a.arm] ?? SERIES[3]}>
                <path d={`${d}L${sx(hi)},${H - B}L${sx(lo)},${H - B}Z`} fill="currentColor" opacity={a.is_control ? 0.08 : 0.12} />
                <path d={d} fill="none" stroke="currentColor" strokeWidth={2} strokeDasharray={a.is_control ? "5 4" : undefined} />
                {/* Hit target wider than the 2px line. */}
                <path d={d} fill="none" stroke="transparent" strokeWidth={14} className="cursor-default">
                  <title>{`${armName(a)}: ${pct(m[i].mean)} (95 % ≈ ${pct(m[i].lo)}–${pct(m[i].hi)})`}</title>
                </path>
              </g>
            );
          })}
        </g>
        {labels.map(
          (p, i) =>
            p && (
              <text key={arms[i].arm} x={p.x} y={p.y} textAnchor="middle" className="fill-foreground text-[11px] font-medium">
                {arms[i].is_control ? t("official") : LETTERS[arms[i].arm - 1]}
              </text>
            ),
        )}
      </svg>
      <figcaption className="sr-only">{t("chartCaption")}</figcaption>
    </figure>
  );
}

// ---------- arms table (also the chart's legend and accessible data view) ----------

export function ArmsTable({ exp }: { exp: VariantExperiment }) {
  const t = useTranslations("experiments");
  const format = useFormatter();
  const armName = useArmName();
  const pct = (n: number, d = 1) => format.number(n, { style: "percent", maximumFractionDigits: d });
  const lift = (n: number) => `${n >= 0 ? "+" : "−"}${format.number(Math.abs(n), { style: "percent", maximumFractionDigits: 1 })}`;

  return (
    <div className="-mx-4 overflow-x-auto sm:-mx-5">
      <table className="w-full min-w-[720px] text-sm">
        <thead className="border-y bg-muted/40 text-xs text-muted-foreground">
          <tr>
            <th scope="col" className="px-4 py-2 text-start font-medium sm:ps-5">{t("table.arm")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium">{t("table.visitors")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium">{t("table.rate", { goal: exp.goal })}</th>
            <th scope="col" className="w-40 px-4 py-2 text-start font-medium">{t("table.pBest")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium sm:pe-5">{t("table.lift")}</th>
          </tr>
        </thead>
        <tbody className="divide-y">
          {exp.arms.map((a) => {
            const won = exp.winner === a.arm && exp.status !== "running";
            return (
              <tr key={a.arm} className={cn("align-top", won && "bg-success/5")}>
                <td className="px-4 py-2.5 sm:ps-5">
                  <div className="flex items-center gap-2">
                    <span aria-hidden className={cn("inline-block h-0.5 w-4 shrink-0 rounded-full bg-current", SERIES[a.arm] ?? SERIES[3], a.is_control && "opacity-70")} />
                    <span className="font-medium">{armName(a)}</span>
                    {!a.is_control && <span className="text-xs text-muted-foreground">{TONES.includes(a.tone as Tone) ? t(`tone.${a.tone as Tone}`) : a.tone}</span>}
                    {won && (
                      <Badge variant="secondary" className="gap-1 border-0 bg-success/15 text-success-foreground">
                        <Trophy className="size-3" /> {t("winner")}
                      </Badge>
                    )}
                  </div>
                  <p className="mt-1 max-w-xl text-pretty text-muted-foreground" lang={exp.locale}>
                    “{a.value}”
                  </p>
                </td>
                <td className="px-4 py-2.5 text-end tabular-nums">{format.number(a.views)}</td>
                <td className="px-4 py-2.5 text-end tabular-nums">
                  {a.views ? pct(a.rate) : "—"}
                  <div className="text-xs text-muted-foreground">
                    {format.number(a.conversions)} / {format.number(a.views)}
                  </div>
                </td>
                <td className="px-4 py-2.5">
                  <div className="flex items-center gap-2">
                    <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                      <div
                        className={cn("h-full rounded-full", a.p_best > 0.95 ? "bg-success" : "bg-foreground/50")}
                        style={{ width: `${Math.max(1, a.p_best * 100)}%` }}
                      />
                    </div>
                    <span className="w-12 text-end tabular-nums">{pct(Math.min(a.p_best, 0.999), a.p_best > 0.99 ? 1 : 0)}</span>
                  </div>
                </td>
                <td className="px-4 py-2.5 text-end tabular-nums sm:pe-5">
                  {a.is_control ? (
                    <span className="text-muted-foreground">{t("reference")}</span>
                  ) : (
                    <>
                      <span className={cn(a.p_beats_control > 0.95 ? "text-success-foreground" : a.p_beats_control < 0.05 ? "text-destructive" : "")}>
                        {lift(a.lift)}
                      </span>
                      <div className="text-xs text-muted-foreground">{t("pBeats", { p: pct(Math.min(a.p_beats_control, 0.999), a.p_beats_control > 0.99 ? 1 : 0) })}</div>
                    </>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// ---------- card ----------

const STATUS = {
  running: { icon: CircleDashed, className: "bg-info/15 text-info-foreground" },
  decided: { icon: Trophy, className: "bg-success/15 text-success-foreground" },
  promoted: { icon: Check, className: "bg-muted text-muted-foreground" },
  stopped: { icon: Square, className: "bg-muted text-muted-foreground" },
} as const;

export function ExperimentCard({
  exp,
  busy,
  onAction,
}: {
  exp: VariantExperiment;
  busy: boolean;
  onAction: (action: "promote" | "stop") => void;
}) {
  const t = useTranslations("experiments");
  const format = useFormatter();
  const name = useLocaleName();
  const armName = useArmName();
  const s = STATUS[exp.status];
  const seen = Math.min(...exp.arms.map((a) => a.views));
  const burnIn = seen < exp.min_n;
  const winner = exp.arms.find((a) => a.arm === exp.winner);
  const live = exp.status === "running" || exp.status === "decided";
  const total = exp.arms.reduce((n, a) => n + a.views, 0);

  return (
    <article className="rounded-xl border p-4 sm:p-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate font-mono text-sm font-medium" translate="no">{exp.key}</h3>
            <span className="text-xs text-muted-foreground">
              {name(exp.locale)} <span className="font-mono">{exp.locale}</span>
            </span>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("meta", { goal: exp.goal, pct: exp.explore_pct, n: exp.min_n })} · {t("source")} <span lang="en">“{exp.source}”</span>
          </p>
        </div>
        <Badge variant="secondary" className={cn("gap-1 border-0 font-medium", s.className)}>
          <s.icon className="size-3" /> {t(`status.${exp.status}`)}
        </Badge>
      </header>

      {exp.source_changed && live && (
        <p className="mt-3 rounded-md border border-warning/40 bg-warning/10 p-2.5 text-xs">{t("sourceChanged")}</p>
      )}

      <div className="mt-4">
        {total === 0 ? (
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">{t("noTraffic")}</p>
        ) : (
          <PosteriorChart arms={exp.arms} />
        )}
      </div>

      <div className="mt-4">
        <ArmsTable exp={exp} />
      </div>

      <footer className="mt-4 flex flex-wrap items-center gap-3 border-t pt-4">
        <div className="min-w-[16rem] flex-1 text-xs text-muted-foreground">
          {exp.status === "running" && burnIn && (
            <div className="max-w-xs">
              <div className="flex justify-between">
                <span>{t("burnIn")}</span>
                <span className="tabular-nums">
                  {format.number(seen)} / {format.number(exp.min_n)}
                </span>
              </div>
              <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-info/15">
                <div className="h-full rounded-full bg-info" style={{ width: `${Math.min(100, (seen / exp.min_n) * 100)}%` }} />
              </div>
            </div>
          )}
          <p className={cn(burnIn && exp.status === "running" && "mt-2")}>
            {exp.status === "decided" && winner
              ? winner.is_control
                ? t("explain.officialWins")
                : t("explain.decided", { name: armName(winner), p: format.number(Math.min(winner.p_best, 0.999), { style: "percent", maximumFractionDigits: 1 }) })
              : t(`explain.${exp.status === "running" && burnIn ? "burnIn" : exp.status}`)}
          </p>
        </div>
        {exp.status === "decided" && winner && (
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" disabled={busy}>
                {busy ? <Loader2 className="animate-spin" /> : <Trophy />}{" "}
                {winner.is_control ? t("keepOfficial") : t("promote", { name: armName(winner) })}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{winner.is_control ? t("keepOfficial") : t("promote", { name: armName(winner) })}</AlertDialogTitle>
                <AlertDialogDescription>
                  {winner.is_control ? t("keepBody") : t("promoteBody", { key: exp.key, locale: exp.locale, value: winner.value })}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
                <AlertDialogAction onClick={() => onAction("promote")}>{t("confirm")}</AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
        {live && (
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant="ghost" disabled={busy}>
                <X /> {t("stop")}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{t("stopTitle")}</AlertDialogTitle>
                <AlertDialogDescription>{t("stopBody")}</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
                <AlertDialogAction variant="destructive" onClick={() => onAction("stop")}>
                  {t("stop")}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
      </footer>
    </article>
  );
}
