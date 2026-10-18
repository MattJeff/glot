"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderKanban, Plus } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { EmptyState, ErrorState, PageHeader, ProgressBar } from "@/components/common";
import { CreateProjectDialog } from "@/components/create-project-dialog";
import { projectHealth } from "@/components/gamification/project-progress";
import { ProgressRing } from "@/components/gamification/progress-ring";
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
        <ul className="stagger grid gap-4 sm:grid-cols-2 xl:grid-cols-3" aria-label={t("title")}>
          {projects.data.map((p, i) => (
            <li key={p.id} style={{ "--i": i } as React.CSSProperties} className="flex">
              <ProjectCard project={p} />
            </li>
          ))}
        </ul>
      )}
      <CreateProjectDialog open={open} onOpenChange={setOpen} />
    </div>
  );
}

function ProjectCard({ project: p }: { project: Project }) {
  const t = useTranslations("projects");
  const format = useFormatter();
  const tg = useTranslations("gamification");
  const health = projectHealth(p);
  const qc = useQueryClient();
  // Seed the overview with what we already have, then refresh in the background.
  const prefetch = () => {
    if (!qc.getQueryData(qk.project(p.slug))) qc.setQueryData(qk.project(p.slug), p);
  };
  return (
    <Link
      href={`/app/${p.slug}`}
      onMouseEnter={prefetch}
      onFocus={prefetch}
      className="group flex w-full flex-col rounded-xl border bg-card p-5 shadow-(--shadow-card) transition-[border-color,box-shadow,translate] duration-200 ease-out hover:border-foreground/20 hover:shadow-(--shadow-raised) focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none motion-safe:hover:-translate-y-0.5"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate font-medium">{p.name}</h2>
          <p className="truncate font-mono text-xs text-muted-foreground">
            {p.slug} · {p.source_locale}
          </p>
        </div>
        {health !== null && (
          <ProgressRing
            value={health}
            size={40}
            label={tg("healthLabel", { score: Math.round(health) })}
          >
            <span className="text-[11px] font-semibold tabular-nums">{Math.round(health)}</span>
          </ProgressRing>
        )}
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
              <ProgressBar
                total={p.stats.keys}
                translated={s.translated}
                reviewed={s.reviewed}
                stale={s.stale}
                label={l}
              />
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
