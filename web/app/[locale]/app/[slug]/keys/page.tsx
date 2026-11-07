"use client";

import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { ErrorState, PageHeader } from "@/components/common";
import { SegmentsEditor } from "@/components/edge/segments";
import { KeysEditor } from "@/components/keys-editor";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { useProject, useSlug } from "@/lib/hooks";

/** Keys (catalog entries) or, with ?view=segments, glot Edge segments (rendered text without a key). */
function Keys() {
  const t = useTranslations("edge.segments");
  const slug = useSlug();
  const project = useProject(slug);
  const segments = useSearchParams().get("view") === "segments";
  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  if (!segments) return <KeysEditor project={project.data} />;
  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button asChild variant="outline" size="sm">
            <Link href={`/app/${slug}/keys`}>{t("viewKeys")}</Link>
          </Button>
        }
      />
      <SegmentsEditor project={project.data} />
    </div>
  );
}

export default function KeysPage() {
  return (
    <Suspense>
      <Keys />
    </Suspense>
  );
}
