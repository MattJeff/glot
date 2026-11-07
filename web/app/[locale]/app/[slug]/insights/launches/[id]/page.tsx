"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Link2Off, Share2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useParams } from "next/navigation";
import { toast } from "sonner";
import { CopyButton, ErrorState } from "@/components/common";
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
      {l ? <LaunchStoryView story={l} /> : <Skeleton className="h-96 rounded-xl" />}
    </div>
  );
}
