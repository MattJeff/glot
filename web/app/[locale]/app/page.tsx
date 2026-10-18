"use client";

import { useQuery } from "@tanstack/react-query";
import { FolderKanban, Plus } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { EmptyState, ErrorState, PageHeader, ProgressBar } from "@/components/common";
import { CreateProjectDialog } from "@/components/create-project-dialog";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk, type Project } from "@/lib/api";

export default function ProjectsPage() {
  const t = useTranslations("projects");
  const [open, setOpen] = useState(false);
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects });

  const newButton = (
    <Button onClick={() => setOpen(true)}>
      <Plus /> {t("create.title")}
    </Button>
  );

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader title={t("title")} subtitle={t("subtitle")} actions={projects.data?.length ? newButton : null} />
      {projects.isPending ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-44 rounded-xl" />
          ))}
        </div>
      ) : projects.isError ? (
        <ErrorState error={projects.error} onRetry={() => projects.refetch()} />
      ) : projects.data.length === 0 ? (
        <EmptyState
          icon={FolderKanban}
          title={t("empty.title")}
          body={t("empty.body")}
          action={
            <Button onClick={() => setOpen(true)}>
              <Plus /> {t("empty.cta")}
            </Button>
          }
        />
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {projects.data.map((p) => (
            <ProjectCard key={p.id} project={p} />
          ))}
        </div>
      )}
      <CreateProjectDialog open={open} onOpenChange={setOpen} />
    </div>
  );
}

function ProjectCard({ project: p }: { project: Project }) {
  const t = useTranslations("projects");
  const format = useFormatter();
  return (
    <Link
      href={`/app/${p.slug}`}
      className="group flex flex-col rounded-xl border bg-card p-5 transition-colors hover:border-foreground/20 hover:bg-accent/30 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate font-medium">{p.name}</h2>
          <p className="truncate font-mono text-xs text-muted-foreground">{p.slug}</p>
        </div>
        <span className="shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
          {p.source_locale}
        </span>
      </div>
      <p className="mt-3 text-xs text-muted-foreground">
        {t("keysCount", { count: p.stats.keys })} · {t("localesCount", { count: p.target_locales.length })}
      </p>
      <div className="mt-4 space-y-2">
        {p.target_locales.slice(0, 5).map((l) => {
          const s = p.stats.locales[l] ?? { translated: 0, reviewed: 0, stale: 0, issues: 0 };
          const ratio = p.stats.keys ? s.translated / p.stats.keys : 0;
          return (
            <div key={l} className="flex items-center gap-3">
              <span className="w-10 shrink-0 font-mono text-xs">{l}</span>
              <ProgressBar total={p.stats.keys} translated={s.translated} reviewed={s.reviewed} stale={s.stale} />
              <span className="w-10 shrink-0 text-end font-mono text-xs text-muted-foreground tabular-nums">
                {format.number(ratio, { style: "percent", maximumFractionDigits: 0 })}
              </span>
            </div>
          );
        })}
      </div>
    </Link>
  );
}
