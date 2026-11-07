"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Lock } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useSyncExternalStore } from "react";
import { ErrorState, PageHeader } from "@/components/common";
import { ExperimentWizard } from "@/components/experiments/wizard/wizard";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";

const noop = () => () => {};

export default function NewExperimentPage() {
  const t = useTranslations("xpWizard");
  const tl = useTranslations("xpLifecycle");
  const slug = useSlug();
  const project = useProject(slug);
  // `?edit=<id>`: the same wizard on an existing server draft (detail page → Edit).
  const editId = useSearchParams().get("edit");
  const edit = useQuery({ queryKey: qk.xp(slug, editId ?? ""), queryFn: () => api.xp(slug, editId!), enabled: !!editId, staleTime: 0 });
  const back = editId ? `/app/${slug}/experiments/${editId}` : `/app/${slug}/experiments`;
  // The draft is read from localStorage: render the wizard on the client only (no hydration mismatch).
  const mounted = useSyncExternalStore(noop, () => true, () => false);

  return (
    <div className="mx-auto max-w-6xl">
      <Link
        href={back}
        className="mb-3 inline-flex min-h-6 items-center gap-1 rounded-sm text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-4 rtl:-scale-x-100" aria-hidden /> {editId ? tl("backToExperiment") : t("backToList")}
      </Link>
      <PageHeader
        title={editId ? tl("editTitle", { name: edit.data?.name ?? "…" }) : t("title")}
        subtitle={editId ? tl("editSubtitle") : t("subtitle")}
      />
      {project.isError || edit.isError ? (
        <ErrorState error={project.error ?? edit.error} onRetry={() => (edit.isError ? edit.refetch() : project.refetch())} />
      ) : !project.data || !mounted || (editId && !edit.data) ? (
        <div className="space-y-6">
          <Skeleton className="h-12 rounded-lg" />
          <Skeleton className="h-[28rem] rounded-xl" />
        </div>
      ) : edit.data && edit.data.status !== "draft" ? (
        <div className="flex items-start gap-3 rounded-xl border bg-muted/30 p-4 text-sm">
          <Lock className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
          <p className="text-pretty text-muted-foreground">{tl("locked")}</p>
        </div>
      ) : (
        <ExperimentWizard key={editId ?? "new"} project={project.data} editing={edit.data} />
      )}
    </div>
  );
}
