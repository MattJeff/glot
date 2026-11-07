"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CheckCircle2, Link2Off, Rocket, Share2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useParams } from "next/navigation";
import { toast } from "sonner";
import { CopyButton, ErrorState, useLocaleName } from "@/components/common";
import { LaunchStoryView } from "@/components/launch-story";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk } from "@/lib/api";
import { useOrigin, useSlug } from "@/lib/hooks";

export default function LaunchPage() {
  const t = useTranslations("launches");
  const slug = useSlug();
  const { id } = useParams<{ id: string }>();
  const origin = useOrigin();
  const name = useLocaleName();
  const qc = useQueryClient();
  const key = [...qk.launches(slug), id];
  const q = useQuery({ queryKey: key, queryFn: () => api.launch(slug, id), refetchInterval: 60_000 });
  const share = useMutation({
    mutationFn: async (on: boolean) => {
      await (on ? api.shareLaunch(slug, id) : api.revokeLaunch(slug, id));
    },
    onSuccess: (_, on) => {
      qc.invalidateQueries({ queryKey: qk.launches(slug) });
      toast.success(t(on ? "share.created" : "share.revoked"));
    },
  });

  const rollout = useMutation({
    mutationFn: () => api.rolloutLaunch(slug, id),
    onSuccess: (l) => {
      qc.setQueryData(key, l);
      qc.invalidateQueries({ queryKey: qk.launches(slug) });
      toast.success(t("rollout.done", { language: name(l.locale) }));
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const l = q.data;
  const url = l?.share_token && origin ? `${origin}/case/${l.share_token}` : null;
  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Link href={`/app/${slug}/insights/launches`} className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground">
          <ArrowLeft className="size-3 rtl:rotate-180" /> {t("title")}
        </Link>
        {l && !url && (
          <Button size="sm" variant="outline" disabled={share.isPending} onClick={() => share.mutate(true)}>
            <Share2 /> {t("share.button")}
          </Button>
        )}
      </div>
      {url && (
        <div className="rounded-xl border bg-muted/30 p-4">
          <div className="text-sm font-medium">{t("share.title")}</div>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("share.body")}</p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border bg-background px-2 py-1.5 text-xs" translate="no">
              {url}
            </code>
            <CopyButton value={url} />
            <Button size="sm" variant="ghost" disabled={share.isPending} onClick={() => share.mutate(false)}>
              <Link2Off /> {t("share.revoke")}
            </Button>
          </div>
        </div>
      )}
      {l?.mode === "holdout" && l.status === "ready" && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-success/40 bg-success/10 p-4">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 text-sm font-medium">
              <CheckCircle2 className="size-4 text-success-foreground" /> {t("rollout.readyTitle")}
            </div>
            <p className="mt-0.5 text-xs text-muted-foreground">{t("rollout.readyBody", { min: l.min_conversions, fallback: name(l.fallback) })}</p>
          </div>
          <Button size="sm" disabled={rollout.isPending} onClick={() => rollout.mutate()}>
            <Rocket /> {t("rollout.confirm")}
          </Button>
        </div>
      )}
      {l?.mode === "holdout" && (l.status === "done" || l.status === "invalid") && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-muted/30 p-4">
          <div className="min-w-0">
            <div className="text-sm font-medium">{t("rollout.endedTitle")}</div>
            <p className="mt-0.5 text-xs text-muted-foreground">{t("rollout.endedBody", { fallback: name(l.fallback) })}</p>
          </div>
          <Button size="sm" variant="outline" disabled={rollout.isPending} onClick={() => rollout.mutate()}>
            <Rocket /> {t("rollout.confirm")}
          </Button>
        </div>
      )}
      {l?.mode === "holdout" && l.status === "measuring" && l.blockers.length > 0 && (
        <div className="rounded-xl border p-4 text-xs text-muted-foreground">
          <div className="font-medium text-foreground">{t(l.auto_rollout ? "rollout.pendingAuto" : "rollout.pending")}</div>
          <ul className="mt-1 list-disc space-y-0.5 ps-4">
            {l.blockers.map((b) => (
              <li key={b}>{t(`blocker.${b}`, { min: l.min_conversions, n: l.min_n })}</li>
            ))}
          </ul>
        </div>
      )}
      {l ? <LaunchStoryView story={l} /> : <Skeleton className="h-96 rounded-xl" />}
    </div>
  );
}
