"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { ErrorState } from "@/components/common";
import { Reports } from "@/components/review/reports";
import { Workspace } from "@/components/review/workspace";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, qk, type Annotation } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";

type Tab = "annotations" | "reports";

/** Review: the team annotates the live site ("Annotations"); end users' wording reports live in "Reports". */
export default function ReviewPage() {
  const t = useTranslations("reviewPage");
  const slug = useSlug();
  const project = useProject(slug);
  const qc = useQueryClient();
  const [tab, setTab] = useState<Tab>("annotations");
  const [converted, setConverted] = useState<string | undefined>();
  const feedback = useQuery({ queryKey: qk.feedback(slug), queryFn: () => api.feedback(slug), enabled: !!slug });
  const openReports = feedback.data?.groups.filter((g) => g.status === "open" || g.status === "proposed").length ?? 0;
  const convert = useMutation({
    mutationFn: (groupId: string) => api.annotationFromFeedback(slug, groupId),
    onSuccess: (a: Annotation) => {
      qc.setQueryData(qk.annotation(slug, a.id), a);
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
      qc.invalidateQueries({ queryKey: qk.feedback(slug) });
      toast.success(t("converted"));
      setTab("annotations");
      setConverted(a.id);
    },
  });

  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)} className="-mx-4 -my-6 flex h-[calc(100dvh-3rem)] flex-col gap-0 sm:-mx-6 lg:-mx-8">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b px-4 py-2.5 sm:px-6">
        <div className="flex min-w-0 items-baseline gap-2">
          <h1 className="text-base font-semibold">{t("title")}</h1>
          <p className="hidden truncate text-xs text-muted-foreground xl:block">{t("subtitle")}</p>
        </div>
        <TabsList className="ms-auto sm:ms-0">
          <TabsTrigger value="annotations">{t("tabs.annotations")}</TabsTrigger>
          <TabsTrigger value="reports">
            {t("tabs.reports")}
            {openReports > 0 && <span className="text-muted-foreground tabular-nums">{openReports}</span>}
          </TabsTrigger>
        </TabsList>
      </div>
      <TabsContent value="annotations" className="flex min-h-0 flex-1 flex-col">
        {project.data ? <Workspace key={converted} project={project.data} initial={converted} /> : <Skeleton className="m-4 h-[60vh] rounded-xl" />}
      </TabsContent>
      <TabsContent value="reports" className="min-h-0 flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <Reports slug={slug} onAnnotate={(g) => convert.mutate(g.id)} />
      </TabsContent>
    </Tabs>
  );
}
