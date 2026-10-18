"use client";

import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { useLocaleName } from "@/components/common";
import { AnimatedNumber, ProgressRing } from "@/components/gamification/progress-ring";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Link } from "@/i18n/navigation";
import type { Project } from "@/lib/api";
import { useProject } from "@/lib/hooks";
import { cn } from "@/lib/utils";

const EMPTY = { translated: 0, reviewed: 0, stale: 0, issues: 0 };

/** Per-locale coverage % and a 0–100 health (mean of coverage and validation pass rate). */
export function localeScores(p: Project) {
  return p.target_locales.map((l) => {
    const s = p.stats.locales[l] ?? EMPTY;
    const coverage = p.stats.keys ? (s.translated / p.stats.keys) * 100 : 0;
    const valid = s.translated ? (1 - s.issues / s.translated) * 100 : null;
    return { locale: l, coverage, health: valid === null ? coverage : (coverage + valid) / 2, ...s };
  });
}

export function projectHealth(p: Project) {
  const rows = localeScores(p);
  return rows.length && p.stats.keys ? rows.reduce((n, r) => n + r.health, 0) / rows.length : null;
}

// --- Celebrations opt-out (ux-brief §2.3: every gamified element can be turned off) ---
const PREF = "glot:celebrations";
const listeners = new Set<() => void>();
const read = () => {
  try {
    return localStorage.getItem(PREF) !== "off";
  } catch {
    return true;
  }
};
export function useCelebrationsEnabled() {
  const on = useSyncExternalStore(
    (cb) => (listeners.add(cb), () => listeners.delete(cb)),
    read,
    () => true,
  );
  const set = (v: boolean) => {
    try {
      localStorage.setItem(PREF, v ? "on" : "off");
    } catch {
      /* private mode: preference just won't persist */
    }
    listeners.forEach((l) => l());
  };
  return [on, set] as const;
}

/** A short, discreet particle burst from the bottom-end corner (where toasts appear). No-op with reduced motion. */
function burst() {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const root = document.createElement("div");
  root.setAttribute("aria-hidden", "true");
  root.style.cssText = "position:fixed;inset-inline-end:48px;bottom:72px;pointer-events:none;z-index:100";
  const colors = ["var(--brand)", "var(--success)", "var(--info)", "var(--warning)"];
  for (let i = 0; i < 18; i++) {
    const a = (Math.PI * 2 * i) / 18 + Math.random() * 0.3;
    const d = 40 + Math.random() * 50;
    const s = document.createElement("span");
    s.style.cssText = `position:absolute;width:6px;height:6px;border-radius:${i % 2 ? "2px" : "50%"};background:${colors[i % 4]};--dx:${Math.cos(a) * d}px;--dy:${Math.sin(a) * d - 30}px;animation:var(--animate-burst);animation-delay:${i * 8}ms`;
    root.appendChild(s);
  }
  document.body.appendChild(root);
  setTimeout(() => root.remove(), 1400);
}

/**
 * Watches the active project's stats and celebrates real milestones once:
 * a locale reaching 100 %, and validation errors dropping to 0.
 * Milestones already reached the first time a project is seen are recorded silently.
 */
export function ProjectCelebrations({ slug }: { slug: string }) {
  const t = useTranslations("gamification");
  const localeName = useLocaleName();
  const project = useProject(slug);
  const [enabled] = useCelebrationsEnabled();
  const p = project.data;

  useEffect(() => {
    if (!p || !p.stats.keys) return;
    const key = `glot:milestones:${p.slug}`;
    let seen: string[] | null = null;
    try {
      seen = JSON.parse(localStorage.getItem(key) ?? "null");
    } catch {
      /* ignore */
    }
    const rows = localeScores(p);
    const now = rows.filter((r) => r.translated >= p.stats.keys).map((r) => `complete:${r.locale}`);
    const translated = rows.some((r) => r.translated > 0);
    if (translated && rows.every((r) => r.issues === 0)) now.push("zero-errors");
    const fresh = seen ? now.filter((m) => !seen.includes(m)) : [];
    // Forget milestones that regressed, so reaching them again celebrates again.
    try {
      localStorage.setItem(key, JSON.stringify(now));
    } catch {
      /* ignore */
    }
    if (!enabled || !fresh.length) return;
    // One toast per kind of milestone, even when several locales finish in the same job.
    if (fresh.includes("zero-errors")) toast.success(t("celebrate.zeroErrors"), { description: t("celebrate.zeroErrorsBody") });
    const done = fresh.filter((m) => m.startsWith("complete:")).map((m) => localeName(m.slice("complete:".length)));
    if (done.length)
      toast.success(
        done.length === 1 ? t("celebrate.complete", { language: done[0] }) : t("celebrate.completeMany", { count: done.length }),
        { description: done.length === 1 ? t("celebrate.completeBody", { count: p.stats.keys }) : done.join(" · ") },
      );
    burst();
  }, [p, enabled, t, localeName]);

  return null;
}

/** Sidebar card: project health + one ring per target locale (links to that locale's keys). */
export function SidebarProjectProgress({ slug }: { slug: string }) {
  const t = useTranslations("gamification");
  const format = useFormatter();
  const localeName = useLocaleName();
  const p = useProject(slug).data;
  if (!p || !p.stats.keys || !p.target_locales.length) return null;
  const health = projectHealth(p) ?? 0;
  const tone = health >= 90 ? "text-success-foreground" : health >= 70 ? "text-warning-foreground" : "text-destructive";
  const pct = (n: number) => format.number(n / 100, { style: "percent", maximumFractionDigits: 0 });

  return (
    <div className="mx-2 mt-2 rounded-lg border bg-background/60 p-2.5">
      <Tooltip>
        <TooltipTrigger asChild>
          <Link href={`/app/${slug}`} className="flex items-baseline justify-between gap-2 rounded-sm">
            <span className="text-xs text-muted-foreground">{t("health")}</span>
            <span dir="ltr" className={cn("text-sm font-semibold", tone)}>
              <AnimatedNumber value={Math.round(health)} format={(n) => format.number(n)} />
              <span className="text-xs font-normal text-muted-foreground">/100</span>
            </span>
          </Link>
        </TooltipTrigger>
        <TooltipContent side="right" className="max-w-56">{t("healthHint")}</TooltipContent>
      </Tooltip>
      <ul className="mt-2 flex flex-wrap gap-1.5" aria-label={t("localesProgress")}>
        {localeScores(p).map((r) => (
          <li key={r.locale}>
            <Tooltip>
              <TooltipTrigger asChild>
                <Link href={`/app/${slug}/keys?locale=${r.locale}`} className="block rounded-full">
                  <ProgressRing
                    value={r.coverage}
                    size={32}
                    label={t("ringLabel", { language: localeName(r.locale), percent: pct(r.coverage) })}
                  >
                    <span className="font-mono">{r.locale.slice(0, 3)}</span>
                  </ProgressRing>
                </Link>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t("ringTooltip", { language: localeName(r.locale), percent: pct(r.coverage), issues: r.issues })}
              </TooltipContent>
            </Tooltip>
          </li>
        ))}
      </ul>
    </div>
  );
}
