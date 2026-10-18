"use client";

import { Suspense } from "react";
import { ErrorState } from "@/components/common";
import { KeysEditor } from "@/components/keys-editor";
import { Skeleton } from "@/components/ui/skeleton";
import { useProject, useSlug } from "@/lib/hooks";

export default function KeysPage() {
  const project = useProject(useSlug());
  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  return (
    <Suspense>
      <KeysEditor project={project.data} />
    </Suspense>
  );
}
