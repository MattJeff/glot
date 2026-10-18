"use client";

import { useQueryClient } from "@tanstack/react-query";
import { CircleCheck, CircleX, Clock, Loader2 } from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useEffect } from "react";
import { toast } from "sonner";
import { Progress } from "@/components/ui/progress";
import { api, qk, type Job } from "@/lib/api";
import { cn } from "@/lib/utils";

export const isActive = (j: Job) => j.status === "queued" || j.status === "running";

/** Live progress over SSE; patches the job inside the project's job list. */
function useJobEvents(slug: string, job: Job) {
  const qc = useQueryClient();
  const t = useTranslations("jobs");
  const active = isActive(job);
  useEffect(() => {
    if (!active) return;
    const es = new EventSource(api.jobEventsUrl(job.id));
    const patch = (next: Job) =>
      qc.setQueryData<Job[]>(qk.jobs(slug), (jobs) => jobs?.map((j) => (j.id === next.id ? next : j)));
    es.addEventListener("progress", (e) => {
      try {
        patch(JSON.parse((e as MessageEvent<string>).data));
      } catch {
        /* ignore malformed frame */
      }
    });
    es.addEventListener("done", (e) => {
      es.close();
      // Announce the outcome once (sonner toasts are aria-live); progress ticks stay silent.
      try {
        const last: Job = JSON.parse((e as MessageEvent<string>).data);
        if (last.status === "failed") toast.error(t("failed"), { description: last.error ?? undefined });
        else toast.success(t("finished", { count: last.progress.done }));
      } catch {
        /* no payload: the refetch below still updates the list */
      }
      qc.invalidateQueries({ queryKey: qk.jobs(slug) });
      qc.invalidateQueries({ queryKey: qk.project(slug) });
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      qc.invalidateQueries({ queryKey: qk.issues(slug) });
    });
    // On error the list query falls back to polling (see refetchInterval in callers).
    es.onerror = () => es.close();
    return () => es.close();
  }, [active, job.id, slug, qc, t]);
}

const ICON = {
  queued: <Clock className="size-4 text-muted-foreground" />,
  running: <Loader2 className="size-4 animate-spin text-info-foreground" />,
  done: <CircleCheck className="size-4 text-success-foreground" />,
  failed: <CircleX className="size-4 text-destructive" />,
};

export function JobRow({ slug, job, compact }: { slug: string; job: Job; compact?: boolean }) {
  const t = useTranslations("jobs");
  const ts = useTranslations("jobStatus");
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  useJobEvents(slug, job);
  const { done, total } = job.progress;
  const pct = total ? (done / total) * 100 : job.status === "done" ? 100 : 0;

  return (
    <div className="flex items-center gap-4 px-4 py-3 transition-colors hover:bg-accent/30">
      {ICON[job.status]}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2 text-sm">
          <span className="font-medium">{t(`kind.${job.kind}`)}</span>
          <span className="font-mono text-xs text-muted-foreground">#{job.id.slice(0, 7)}</span>
          <span className="text-xs text-muted-foreground">{ts(job.status)}</span>
        </div>
        <div className="mt-1.5 flex items-center gap-2">
          <Progress
            value={pct}
            className={cn("h-1.5", !compact && "max-w-md", job.status === "done" && "[&>*]:bg-success", job.status === "failed" && "[&>*]:bg-destructive")}
            aria-label={`${t(`kind.${job.kind}`)} #${job.id.slice(0, 7)}: ${ts(job.status)}`}
          />
          <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
            {t("progress", { done: format.number(done), total: format.number(total) })}
          </span>
        </div>
        {job.error && <p className="mt-1 text-xs break-words text-destructive">{job.error}</p>}
      </div>
      <span className="shrink-0 font-mono text-xs tabular-nums">
        {format.number(job.cost_usd, { style: "currency", currency: "USD", maximumFractionDigits: 4 })}
      </span>
      <span
        className={cn("w-24 shrink-0 text-end text-xs text-muted-foreground", compact && "hidden sm:block")}
        title={format.dateTime(new Date(job.created_at), { dateStyle: "medium", timeStyle: "short" })}
      >
        {isActive(job) ? (
          <span className="inline-flex items-center gap-1.5 text-info-foreground">
            <span className="size-1.5 animate-pulse rounded-full bg-info" /> {t("live")}
          </span>
        ) : (
          format.relativeTime(new Date(job.finished_at ?? job.created_at), now)
        )}
      </span>
    </div>
  );
}
