"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleCheck, CircleX, Languages, Loader2, RotateCw, TriangleAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { api, qk, type Review, type Verdict } from "@/lib/api";
import { cn } from "@/lib/utils";

const VERDICT_STYLE: Record<Verdict, string> = {
  same: "bg-success/12 text-success-foreground",
  nuance: "bg-warning/15 text-warning-foreground",
  different: "bg-destructive/10 text-destructive",
};
const VERDICT_ICON = { same: CircleCheck, nuance: TriangleAlert, different: CircleX } as const;

export function VerdictBadge({ verdict, className }: { verdict: Verdict; className?: string }) {
  const t = useTranslations("review.verdict");
  const Icon = VERDICT_ICON[verdict];
  return (
    <Badge variant="secondary" className={cn("h-5 gap-1 border-0 px-1.5 text-[11px] font-medium", VERDICT_STYLE[verdict], className)}>
      <Icon className="size-3" aria-hidden />
      {t(verdict)}
    </Badge>
  );
}

/** Cached review for (key, locale), if one was fetched or produced by "Review this locale". Never triggers an LLM call. */
export function useCachedReview(slug: string, key: string, locale: string) {
  return useQuery({
    queryKey: qk.review(slug, key, locale),
    queryFn: () => api.reviewKey(slug, key, locale),
    enabled: false,
    staleTime: Infinity,
  }).data;
}

export function seedReviews(qc: ReturnType<typeof useQueryClient>, slug: string, reviews: Review[]) {
  for (const r of reviews) qc.setQueryData(qk.review(slug, r.key, r.locale), r);
}

/** Back-translation + verdict + explanation for one translation, computed on demand. */
export function ReviewBlock({
  slug,
  keyName,
  locale,
  sourceLocale,
  hasTranslation,
  onApplied,
}: {
  slug: string;
  keyName: string;
  locale: string;
  sourceLocale: string;
  hasTranslation: boolean;
  onApplied?: () => void;
}) {
  const t = useTranslations("review");
  const qc = useQueryClient();
  const review = useCachedReview(slug, keyName, locale);
  const run = useMutation({
    mutationFn: () => api.reviewKey(slug, keyName, locale),
    onSuccess: (r) => qc.setQueryData(qk.review(slug, keyName, locale), r),
  });
  const apply = useMutation({
    mutationFn: (value: string) => api.putTranslation(slug, keyName, locale, value),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      qc.invalidateQueries({ queryKey: qk.project(slug) });
      qc.removeQueries({ queryKey: qk.review(slug, keyName, locale) });
      toast.success(t("applied"));
      onApplied?.();
    },
  });

  if (!hasTranslation) return null;
  if (!review) {
    return (
      <Button variant="outline" size="xs" onClick={() => run.mutate()} disabled={run.isPending}>
        {run.isPending ? <Loader2 className="animate-spin" /> : <Languages />}
        {run.isPending ? t("running") : t("backTranslate")}
      </Button>
    );
  }
  return (
    <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
      <div className="flex items-center gap-2">
        <VerdictBadge verdict={review.verdict} />
        <span className="text-xs text-muted-foreground">{t("backTranslation", { locale: sourceLocale })}</span>
        <Button
          variant="ghost"
          size="icon-xs"
          className="ms-auto"
          aria-label={t("refresh")}
          disabled={run.isPending}
          onClick={() => run.mutate()}
        >
          <RotateCw className={cn(run.isPending && "animate-spin")} />
        </Button>
      </div>
      <p dir="auto" lang={sourceLocale} className="text-sm italic">
        {review.back_translation}
      </p>
      <p className="text-xs text-muted-foreground">{review.explanation}</p>
      {review.suggestion && review.verdict !== "same" && (
        <div className="rounded-md border border-dashed p-2">
          <p className="text-[11px] font-medium text-muted-foreground">{t("suggestion")}</p>
          <p dir="auto" lang={locale} className="mt-0.5 text-sm">
            {review.suggestion}
          </p>
          <Button size="xs" className="mt-2" disabled={apply.isPending} onClick={() => apply.mutate(review.suggestion!)}>
            {apply.isPending && <Loader2 className="animate-spin" />}
            {t("apply")}
          </Button>
        </div>
      )}
    </div>
  );
}
