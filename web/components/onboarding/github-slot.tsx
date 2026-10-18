"use client";

// SLOT for the "Connect GitHub" card (owned by the GitHub agent): replace this component's body, keep its props.
// Rendered by the onboarding checklist when the user picks the GitHub path. The checklist ticks "first push"
// by itself as soon as keys reach the project, whatever path sent them.
import { GitBranch } from "lucide-react";
import { useTranslations } from "next-intl";

export function GithubSlot({ slug }: { slug: string }) {
  const t = useTranslations("onboarding.github");
  return (
    <div data-slot="github-connect" data-project={slug} className="flex items-start gap-3 rounded-lg border border-dashed p-4">
      <GitBranch aria-hidden className="mt-0.5 size-5 shrink-0 text-muted-foreground" />
      <div className="text-sm">
        <p className="font-medium">{t("title")}</p>
        <p className="mt-1 text-muted-foreground">{t("soon")}</p>
      </div>
    </div>
  );
}
