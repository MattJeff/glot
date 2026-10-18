"use client";

import { ArrowRight, Rocket, X } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { useOnboarding, usePatchOnboarding } from "./api";
import { Achievements } from "./checklist";
import { NextActionCard } from "./next-action";
import { ProgressRing } from "./progress-ring";

/** Project overview block: setup progress (hideable), next best action, a ring per language, milestones reached. */
export function OnboardingOverview({ slug }: { slug: string }) {
  const t = useTranslations("onboarding.overview");
  const name = useLocaleName();
  const format = useFormatter();
  const ob = useOnboarding(slug);
  const patch = usePatchOnboarding(slug);
  const o = ob.data;
  if (!o) return null;

  const doneCount = o.steps.filter((s) => s.done).length;
  const total = o.steps.length;
  const showSetup = !o.hidden && doneCount < total;

  if (o.keys === 0) {
    return (
      <div className="mt-6 flex flex-col items-center rounded-xl border border-dashed px-6 py-12 text-center">
        <ProgressRing value={(doneCount / total) * 100} size={48} label={t("setupProgress", { done: doneCount, total })}>
          {doneCount}/{total}
        </ProgressRing>
        <h2 className="mt-4 font-medium">{t("emptyTitle")}</h2>
        <p className="mt-1 max-w-md text-sm text-pretty text-muted-foreground">{t("emptyBody")}</p>
        <Button className="mt-5" asChild>
          <Link href={`/app/${slug}/onboarding`}>
            <Rocket aria-hidden />
            {t("start")}
          </Link>
        </Button>
      </div>
    );
  }

  return (
    <section aria-labelledby="ob-overview" className="mt-6 space-y-4 rounded-xl border p-4">
      <div className="flex flex-wrap items-center gap-3">
        {showSetup && (
          <ProgressRing value={(doneCount / total) * 100} size={40} label={t("setupProgress", { done: doneCount, total })}>
            {doneCount}
          </ProgressRing>
        )}
        <div className="min-w-0 flex-1">
          <h2 id="ob-overview" className="text-sm font-medium">
            {showSetup ? t("setupTitle") : t("title")}
          </h2>
          {showSetup && <p className="text-xs text-muted-foreground">{t("setupProgress", { done: doneCount, total })}</p>}
        </div>
        <Button variant={showSetup ? "outline" : "ghost"} size="sm" asChild>
          <Link href={`/app/${slug}/onboarding`}>
            {showSetup ? t("continue") : t("guide")}
            <ArrowRight aria-hidden className="rtl:-scale-x-100" />
          </Link>
        </Button>
        {showSetup && (
          <Button variant="ghost" size="icon-sm" aria-label={t("hide")} onClick={() => patch.mutate({ hidden: true })}>
            <X aria-hidden />
          </Button>
        )}
      </div>

      <NextActionCard slug={slug} onboarding={o} secondary />

      {o.locales.length > 0 && (
        <ul className="flex flex-wrap gap-x-5 gap-y-3" aria-label={t("rings")}>
          {o.locales.map((l) => {
            const pct = Math.round((Math.min(l.translated, o.keys) / o.keys) * 100);
            return (
              <li key={l.locale}>
                <Link
                  href={`/app/${slug}/keys?locale=${l.locale}`}
                  className="flex items-center gap-2 rounded-md p-1 pe-2 hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <ProgressRing value={pct} size={32} stroke={3.5} label={t("ring", { language: name(l.locale), value: pct })} />
                  <span className="text-sm leading-tight">
                    <span className="block capitalize">{name(l.locale)}</span>
                    <span className="block font-mono text-xs tabular-nums text-muted-foreground">
                      {format.number(pct / 100, { style: "percent" })}
                    </span>
                  </span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}

      <Achievements o={o} compact />
    </section>
  );
}
