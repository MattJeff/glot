"use client";

import { useMutation } from "@tanstack/react-query";
import { ArrowRight, BookA, CircleX, Globe, Loader2, Sparkles, Upload } from "lucide-react";
import { useTranslations } from "next-intl";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Link, useRouter } from "@/i18n/navigation";
import { api } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { toast } from "sonner";
import { nextAction, type Onboarding } from "./api";

/** "3 keys are missing in Japanese → Translate missing": the single most useful next step. */
/** `secondary` when the page already has a primary action (one primary per screen). */
export function NextActionCard({ slug, onboarding, secondary }: { slug: string; onboarding: Onboarding; secondary?: boolean }) {
  const t = useTranslations("onboarding.next");
  const name = useLocaleName();
  const router = useRouter();
  const message = useErrorMessage();
  const translate = useMutation({
    mutationFn: (locale: string) => api.createJob(slug, { kind: "translate", locales: [locale] }),
    onSuccess: () => {
      toast.success(t("jobStarted"));
      router.push(`/app/${slug}/jobs`);
    },
    onError: (e) => toast.error(message(e)),
  });
  const a = nextAction(onboarding);
  if (!a) return null;

  const base = `/app/${slug}`;
  const view = {
    push: { icon: Upload, text: t("push"), cta: t("pushCta"), href: `${base}/onboarding` },
    translate: {
      icon: Sparkles,
      text: "locale" in a ? t("translate", { count: a.count, language: name(a.locale) }) : "",
      cta: t("translateCta"),
      href: null,
    },
    fix: {
      icon: CircleX,
      text: "locale" in a ? t("fix", { count: a.count, language: name(a.locale) }) : "",
      cta: t("fixCta"),
      href: "locale" in a ? `${base}/keys?locale=${a.locale}` : base,
    },
    glossary: { icon: BookA, text: t("glossary"), cta: t("glossaryCta"), href: `${base}/glossary` },
    cdn: { icon: Globe, text: t("cdn"), cta: t("cdnCta"), href: `${base}/settings` },
  }[a.kind];
  const Icon = view.icon;

  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border border-brand/30 bg-brand/5 px-4 py-3">
      <Icon aria-hidden className="size-4 shrink-0 text-brand" />
      <p className="min-w-0 flex-1 basis-48 text-sm">
        <span className="sr-only">{t("label")} </span>
        {view.text}
      </p>
      {view.href ? (
        <Button size="sm" variant={secondary ? "outline" : "default"} asChild>
          <Link href={view.href}>
            {view.cta}
            <ArrowRight aria-hidden className="rtl:-scale-x-100" />
          </Link>
        </Button>
      ) : (
        <Button
          size="sm"
          variant={secondary ? "outline" : "default"}
          disabled={translate.isPending}
          aria-busy={translate.isPending}
          onClick={() => "locale" in a && translate.mutate(a.locale)}
        >
          {translate.isPending ? <Loader2 aria-hidden className="animate-spin" /> : <Sparkles aria-hidden />}
          {view.cta}
        </Button>
      )}
    </div>
  );
}
