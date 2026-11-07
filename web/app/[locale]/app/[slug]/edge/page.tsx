"use client";

import { useQuery } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { ErrorState, PageHeader } from "@/components/common";
import { edgeApi, edgeKeys } from "@/components/edge/api";
import { Coverage } from "@/components/edge/coverage";
import { Performance, SeoReportView } from "@/components/edge/seo";
import { SegmentsEditor } from "@/components/edge/segments";
import { EdgeSettingsForm } from "@/components/edge/settings";
import { KillSwitch, Setup } from "@/components/edge/setup";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useRouter } from "@/i18n/navigation";
import { useProject, useSlug } from "@/lib/hooks";

const TABS = ["setup", "coverage", "review", "seo", "performance", "settings"] as const;
type Tab = (typeof TABS)[number];

function EdgeView() {
  const t = useTranslations("edge");
  const slug = useSlug();
  const router = useRouter();
  const raw = useSearchParams().get("tab") ?? "";
  const tab: Tab = (TABS as readonly string[]).includes(raw) ? (raw as Tab) : "setup";
  const project = useProject(slug);
  const settings = useQuery({ queryKey: edgeKeys.settings(slug), queryFn: () => edgeApi.settings(slug), enabled: !!slug });

  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (settings.isError) return <ErrorState error={settings.error} onRetry={() => settings.refetch()} />;
  if (!project.data || !settings.data) return <Skeleton className="h-[70vh] rounded-xl" />;

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader title={t("title")} subtitle={t("subtitle")} actions={<KillSwitch slug={slug} settings={settings.data} />} />
      <Tabs value={tab} onValueChange={(v) => router.replace(`/app/${slug}/edge?tab=${v}`)}>
        <TabsList className="mb-6 h-auto flex-wrap">
          {TABS.map((k) => (
            <TabsTrigger key={k} value={k}>
              {t(`tabs.${k}`)}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="setup">
          <Setup project={project.data} />
        </TabsContent>
        <TabsContent value="coverage">
          <Coverage project={project.data} settings={settings.data} />
        </TabsContent>
        <TabsContent value="review">
          <SegmentsEditor project={project.data} queue />
        </TabsContent>
        <TabsContent value="seo">
          <SeoReportView project={project.data} />
        </TabsContent>
        <TabsContent value="performance">
          <Performance slug={slug} />
        </TabsContent>
        <TabsContent value="settings">
          <EdgeSettingsForm slug={slug} settings={settings.data} />
        </TabsContent>
      </Tabs>
    </div>
  );
}

export default function EdgePage() {
  return (
    <Suspense>
      <EdgeView />
    </Suspense>
  );
}
