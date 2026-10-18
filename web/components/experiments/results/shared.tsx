"use client";

import { CircleDashed, Equal, Pin, Trophy } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { Badge } from "@/components/ui/badge";
import type { XpArmResult, XpExperiment, XpResults } from "@/lib/api";
import { cn } from "@/lib/utils";

// Official = neutral reference; variants take categorical slots 1-3 in fixed order
// (same validated light/dark steps as the legacy PosteriorChart in experiments/parts.tsx).
const SERIES = [
  "text-muted-foreground",
  "text-[#2a78d6] dark:text-[#3987e5]",
  "text-[#eb6834] dark:text-[#d95926]",
  "text-[#1baf7a] dark:text-[#199e70]",
];
export const seriesOf = (arm: number) => SERIES[arm] ?? SERIES[3];

/** Swatch that carries the arm's identity next to its (ink-colored) name. Dashed for the official text. */
export function ArmSwatch({ arm }: { arm: number }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block w-4 shrink-0", seriesOf(arm), arm === 0 ? "border-t-2 border-dashed border-current" : "h-0.5 rounded-full bg-current")}
    />
  );
}

export type CellState = "learning" | "winner" | "no_difference" | "personalized";
export type ArmView = XpArmResult & { arm: number };
/** One segment as the UI shows it: experiment state (winner, personalization) + its latest analysis. */
export type CellView = {
  id: string;
  name: string;
  state: CellState;
  winner: number | null;
  views: number;
  arms: ArmView[];
  srm_p: number | null;
  days_left: number | null;
  underpowered: boolean;
};

/** Decisions are frozen below this SRM p-value (stats.rs SRM_P). */
export const SRM_P = 0.001;
// ponytail: "no difference" = every variant's 95 % range sits within ±5 % of official; a server flag can replace it.
const NEGLIGIBLE = 0.05;

const daysSince = (iso: string | null, end: string | null) =>
  iso ? Math.max(0, ((end ? new Date(end) : new Date()).getTime() - new Date(iso).getTime()) / 86_400_000) : 0;

/** Targeted segments in priority order, then "*", merged with their results (if the experiment ever ran). */
export function cellViews(exp: XpExperiment, res: XpResults | null | undefined): CellView[] {
  const days = res?.days ?? daysSince(exp.started_at, exp.ended_at);
  const ids = [...exp.cells.map((c) => c.id), "*"];
  const approved = exp.arms.filter((a) => a.approved).map((a) => a.arm);
  return ids.map((id) => {
    const st = exp.state.find((s) => s.cell === id);
    const r = res?.cells.find((c) => c.name === id) ?? (id === "*" && !exp.cells.length ? res?.global : undefined);
    const arms = r ? approved.filter((a) => r.arms[a]).map((a) => ({ ...r.arms[a], arm: a })) : [];
    const winner = st?.winner ?? r?.winner ?? null;
    const views = r?.views ?? 0;
    const variants = arms.filter((a) => a.arm !== 0);
    const state: CellState = st?.personalized
      ? "personalized"
      : winner != null
        ? "winner"
        : variants.length && days >= 7 && variants.every((a) => a.lift_lo > -NEGLIGIBLE && a.lift_hi < NEGLIGIBLE)
          ? "no_difference"
          : "learning";
    // ponytail: traffic check = visitors needed per arm to see a +20 % lift (≈ 16·(1−p)/(0.2²·p)) vs the cell's
    // pace over 90 days. The estimate endpoint (XP2) does this properly at creation time.
    const base = arms.find((a) => a.arm === 0)?.mean ?? 0;
    const perArmPerDay = days >= 1 && arms.length ? views / days / arms.length : null;
    const needed = base > 0 && base < 1 ? (16 * (1 - base)) / (0.04 * base) : null;
    const underpowered = state === "learning" && perArmPerDay != null && needed != null && perArmPerDay * 90 < needed;
    const left = r?.days_left ?? (state === "learning" && perArmPerDay && needed ? Math.max(0, needed / perArmPerDay - days, 7 - days) : null);
    return {
      id,
      name: id === "*" ? "*" : (exp.cells.find((c) => c.id === id)?.name ?? id),
      state,
      winner,
      views,
      arms,
      srm_p: r?.srm_p ?? null,
      days_left: left != null && left < 90 ? left : null,
      underpowered,
    };
  });
}

export function useXpFormat() {
  const t = useTranslations("experimentResults");
  const format = useFormatter();
  const pct = (n: number, d = 0) => format.number(n, { style: "percent", maximumFractionDigits: d });
  // Probabilities never display as a flat 100 %: the data can't be that sure.
  const prob = (p: number) => pct(Math.min(p, 0.999), p > 0.99 ? 1 : 0);
  const signed = (n: number) => `${n >= 0 ? "+" : "−"}${pct(Math.abs(n), Math.abs(n) < 0.1 ? 1 : 0)}`;
  const money = (n: number, currency: string | null) =>
    currency ? format.number(n, { style: "currency", currency, maximumFractionDigits: 2 }) : format.number(n, { maximumFractionDigits: 2 });
  const cellName = (c: { id: string; name: string }, only: boolean) => (c.id === "*" ? t(only ? "everyone" : "everyoneElse") : c.name);
  return { format, pct, prob, signed, money, cellName };
}

export function useArmLabel(exp: XpExperiment) {
  const t = useTranslations("experimentResults");
  return (arm: number) => {
    const a = exp.arms.find((x) => x.arm === arm);
    return !a || arm === 0 ? t("official") : a.intent;
  };
}

const STATE: Record<CellState, { icon: typeof Trophy; className: string }> = {
  learning: { icon: CircleDashed, className: "bg-info/12 text-info-foreground" },
  winner: { icon: Trophy, className: "bg-success/15 text-success-foreground" },
  no_difference: { icon: Equal, className: "bg-muted text-muted-foreground" },
  personalized: { icon: Pin, className: "bg-brand/12 text-foreground" },
};

/** Cell state: icon + text, never color alone. */
export function CellStateBadge({ state }: { state: CellState }) {
  const t = useTranslations("experimentResults.state");
  const s = STATE[state];
  return (
    <Badge variant="secondary" className={cn("gap-1 border-0 font-medium", s.className)}>
      <s.icon className="size-3" aria-hidden /> {t(state)}
    </Badge>
  );
}
