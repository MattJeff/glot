"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Languages, Lock, Plus, Radar, TrendingDown, TrendingUp } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
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
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk, type Recommendation } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";

/** Job cap: the estimate + 20 % (estimates are a dry run), at least one cent. */
const cap = (cost: number) => Math.max(0.01, Math.ceil(cost * 1.2 * 100) / 100);

/** "Next language": top 3 recommendations from Insights, one click to add the locale and translate it (capped). */
export function NextLanguageCard({ slug, days = 30, goal, emptyState }: { slug: string; days?: number; goal?: string; emptyState?: boolean }) {
  const t = useTranslations("nextLanguage");
  const tc = useTranslations("common");
  const format = useFormatter();
  const name = useLocaleName();
  const qc = useQueryClient();
  const origin = useOrigin() ?? "";
  const [confirm, setConfirm] = useState<Recommendation | null>(null);
  const q = useQuery({ queryKey: [...qk.recommendations(slug), days, goal], queryFn: () => api.recommendations(slug, { days, goal }) });
  const project = useQuery({ queryKey: qk.project(slug), queryFn: () => api.project(slug) });
  const add = useMutation({
    mutationFn: async (r: Recommendation) => {
      const targets = project.data?.target_locales ?? [];
      if (!targets.includes(r.locale)) await api.updateProject(slug, { target_locales: [...targets, r.locale] });
      return api.createJob(slug, { kind: "translate", locales: [r.locale], budget_usd: cap(r.cost_usd) });
    },
    onSuccess: (_, r) => {
      for (const k of [qk.recommendations(slug), qk.project(slug), qk.insights(slug), qk.jobs(slug)]) qc.invalidateQueries({ queryKey: k });
      toast.success(t("started", { language: name(r.locale), cap: usd(cap(r.cost_usd)) }));
      setConfirm(null);
    },
    onError: (e) => toast.error(e.message),
  });

  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD", maximumFractionDigits: n > 0 && n < 0.1 ? 4 : 2 });
  const pct = (n: number) => format.number(n, { style: "percent", maximumFractionDigits: n > 0 && n < 0.1 ? 1 : 0 });
  const num = (n: number, sign = false) =>
    format.number(n, { maximumFractionDigits: Math.abs(n) < 10 ? 1 : 0, signDisplay: sign ? "exceptZero" : "auto" });
  const title = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

  if (q.isError) return null; // ponytail: the pages around it already report API errors.
  if (!q.data || !project.data) return <Skeleton className="h-44 rounded-xl" />;
  const d = q.data;

  if (d.total_visits === 0) {
    if (!emptyState) return null; // Analyses shows its own install panel.
    const tag = `<script src="https://unpkg.com/@glot-i18n/insights/dist/insights.global.js" data-key="${d.public_key}" data-endpoint="${origin}" defer></script>`;
    return (
      <section aria-labelledby="next-language" className="mt-6 rounded-xl border border-dashed p-5 sm:p-6">
        <h2 id="next-language" className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Radar className="size-3.5" aria-hidden /> {t("kicker")}
        </h2>
        <p className="mt-2 font-medium">{t("empty.title")}</p>
        <p className="mt-1 max-w-2xl text-sm text-muted-foreground">{t("empty.body")}</p>
        <pre className="mt-3 overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono text-xs" translate="no">{tag}</pre>
        <p className="mt-3 text-sm text-muted-foreground">{t("empty.track")}</p>
        <pre className="mt-2 overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono text-xs" translate="no">{`glot.track("${d.goal}")`}</pre>
        <Button variant="outline" size="sm" className="mt-4" asChild>
          <Link href={`/app/${slug}/insights`}>{t("empty.open")}</Link>
        </Button>
      </section>
    );
  }

  const [best, ...rest] = d.recommendations;
  const facts = (r: Recommendation) =>
    [
      t("cost", { cost: usd(r.cost_usd), keys: r.keys_to_translate }),
      r.gain_monthly &&
        (r.gain_monthly.high <= 0
          ? t("noGain", { goal: d.goal })
          : t("gain", { mid: num(r.gain_monthly.mid, true), low: num(r.gain_monthly.low), high: num(r.gain_monthly.high), goal: d.goal })),
      r.cost_per_conversion != null && t("perConversion", { cost: usd(r.cost_per_conversion) }),
    ].filter(Boolean) as string[];
  const headline = (r: Recommendation) =>
    t(r.kind === "missing" ? "headline" : "headlineFinish", { language: title(name(r.locale)), share: pct(r.share), coverage: pct(r.coverage) });
  const cta = (r: Recommendation) => t(r.kind === "missing" ? "add" : "finish", { language: name(r.locale) });
  const locked = (
    <span className="inline-flex items-center gap-1">
      <Lock className="size-3" aria-hidden /> {t("gainLocked")} ·{" "}
      <Link href="/app/settings/billing" className="font-medium underline underline-offset-4">
        {t("upgrade")}
      </Link>
    </span>
  );

  return (
    <section aria-labelledby="next-language" className="mt-6 rounded-xl border bg-gradient-to-br from-brand/10 via-background to-background p-5 sm:p-6">
      <h2 id="next-language" className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Languages className="size-3.5" aria-hidden /> {t("kicker")}
      </h2>
      {!d.conversions_tracked && <p className="mb-3 rounded-lg border bg-background/60 px-3 py-2 text-sm">{t("noConversions", { goal: d.goal })}</p>}
      {!best ? (
        <p className="text-sm text-muted-foreground">{t("allServed")}</p>
      ) : (
        <>
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0 max-w-2xl">
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-xl font-semibold tracking-tight text-balance sm:text-2xl">{headline(best)}</p>
                <Confidence r={best} />
              </div>
              <p className="mt-2 text-sm">
                {facts(best).join(" · ")}
                {best.gain_locked && d.conversions_tracked && <> · {locked}</>}
              </p>
              <ul className="mt-2 space-y-0.5 text-sm text-muted-foreground">
                {best.served_now && (
                  <li>{t("servedNow", { locale: name(best.served_now), share: pct(best.served_now_share) })}</li>
                )}
                {d.conversions_tracked && (
                  <li>{t("rates", { fallback: pct(best.fallback.rate), native: pct(best.native.rate), goal: d.goal })}</li>
                )}
                {best.trend != null && (best.reasons.includes("rising") || best.reasons.includes("falling")) && (
                  <li className="flex items-center gap-1">
                    {best.trend > 0 ? <TrendingUp className="size-3.5" aria-hidden /> : <TrendingDown className="size-3.5" aria-hidden />}
                    {t(best.trend > 0 ? "trendUp" : "trendDown", { pct: pct(Math.abs(best.trend)) })}
                  </li>
                )}
                {best.reasons
                  .filter((x) => x !== "rising" && x !== "falling" && x !== "no_conversions")
                  .map((x) => (
                    <li key={x}>{t(`reasons.${x}`)}</li>
                  ))}
              </ul>
            </div>
            <Button onClick={() => setConfirm(best)} disabled={add.isPending}>
              <Plus aria-hidden /> {cta(best)}
            </Button>
          </div>

          {rest.length > 0 && (
            <ul className="mt-4 grid gap-2 sm:grid-cols-2" aria-label={t("runnersUp")}>
              {rest.slice(0, 2).map((r) => (
                <li key={r.locale} className="flex items-center justify-between gap-2 rounded-lg border bg-background/60 px-3 py-2 text-sm">
                  <span className="min-w-0">
                    <span className="flex items-center gap-2 font-medium">
                      <span className="truncate">{headline(r)}</span>
                      <Confidence r={r} />
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {facts(r).join(" · ")}
                      {r.gain_locked && d.conversions_tracked && <> · {locked}</>}
                    </span>
                  </span>
                  <Button size="sm" variant="outline" onClick={() => setConfirm(r)} disabled={add.isPending} aria-label={cta(r)}>
                    <Plus aria-hidden />
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      <p className="mt-3 text-xs text-muted-foreground">{t("footnote", { visits: d.total_visits, days: d.days })}</p>

      <AlertDialog open={!!confirm} onOpenChange={(o) => !o && setConfirm(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirm && t(confirm.kind === "missing" ? "confirm.title" : "confirm.titleFinish", { language: name(confirm.locale) })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm &&
                t("confirm.body", { keys: confirm.keys_to_translate, model: d.model, cost: usd(confirm.cost_usd), cap: usd(cap(confirm.cost_usd)) })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={add.isPending}
              onClick={(e) => {
                e.preventDefault();
                if (confirm) add.mutate(confirm);
              }}
            >
              {t("confirm.cta")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

function Confidence({ r }: { r: Recommendation }) {
  const t = useTranslations("nextLanguage.confidence");
  return (
    <Badge variant={r.confidence === "solid" ? "default" : "outline"} title={t(`${r.confidence}Hint`)}>
      {t(r.confidence)}
    </Badge>
  );
}
