"use client";

import { ArrowLeft } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSyncExternalStore } from "react";
import { ErrorState, PageHeader } from "@/components/common";
import { ExperimentWizard } from "@/components/experiments/wizard/wizard";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { useProject, useSlug } from "@/lib/hooks";

const noop = () => () => {};

export default function NewExperimentPage() {
  const t = useTranslations("xpWizard");
  const slug = useSlug();
  const project = useProject(slug);
  // The draft is read from localStorage: render the wizard on the client only (no hydration mismatch).
  const mounted = useSyncExternalStore(noop, () => true, () => false);

  return (
    <div className="mx-auto max-w-6xl">
      <Link
        href={`/app/${slug}/experiments`}
        className="mb-3 inline-flex min-h-6 items-center gap-1 rounded-sm text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-4 rtl:-scale-x-100" aria-hidden /> {t("backToList")}
      </Link>
      <PageHeader title={t("title")} subtitle={t("subtitle")} />
      {project.isError ? (
        <ErrorState error={project.error} onRetry={() => project.refetch()} />
      ) : !project.data || !mounted ? (
        <div className="space-y-6">
          <Skeleton className="h-12 rounded-lg" />
          <Skeleton className="h-[28rem] rounded-xl" />
        </div>
      ) : (
        <ExperimentWizard project={project.data} />
      )}
    </div>
  );
}
