"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { GitPullRequest, RefreshCcw, ShieldCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useEffect } from "react";
import { toast } from "sonner";
import { PageHeader } from "@/components/common";
import { ghKey, githubApi } from "@/components/github/api";
import { ConnectGitHubCard } from "@/components/github/connect-github-card";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useRouter } from "@/i18n/navigation";
import { useSlug } from "@/lib/hooks";

export function GitHubView() {
  const t = useTranslations("github");
  const slug = useSlug();
  const params = useSearchParams();
  const router = useRouter();
  const qc = useQueryClient();
  const status = useQuery({ queryKey: ghKey.status(slug), queryFn: () => githubApi.status(slug) });
  const unlink = useMutation({
    mutationFn: () => githubApi.unlink(slug),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ghKey.status(slug) });
      toast.success(t("page.disconnected"));
    },
  });

  // Back from github.com: say what happened once, then clean the URL.
  useEffect(() => {
    const msg = params.get("installed")
      ? (["success", t("return.installed")] as const)
      : params.get("pending")
        ? (["info", t("return.pending")] as const)
        : params.get("error")
          ? (["error", t("return.error")] as const)
          : null;
    if (!msg) return;
    toast[msg[0]](msg[1]);
    router.replace(`/app/${slug}/github`);
  }, [params, router, slug, t]);

  const link = status.data?.link;
  return (
    <div className="mx-auto max-w-3xl space-y-8">
      <PageHeader title={t("page.title")} subtitle={t("page.subtitle")} />
      <ConnectGitHubCard slug={slug} />
      <section aria-labelledby="gh-how" className="space-y-3">
        <h2 id="gh-how" className="text-sm font-medium">
          {t("page.how")}
        </h2>
        <ul className="grid gap-3 sm:grid-cols-3">
          {(
            [
              ["pr", GitPullRequest],
              ["safe", ShieldCheck],
              ["updates", RefreshCcw],
            ] as const
          ).map(([k, Icon]) => (
            <li key={k} className="rounded-xl border p-4">
              <Icon className="mb-2 size-5 text-muted-foreground" aria-hidden />
              <p className="text-sm font-medium">{t(`how.${k}.title`)}</p>
              <p className="mt-1 text-sm text-pretty text-muted-foreground">{t(`how.${k}.body`)}</p>
            </li>
          ))}
        </ul>
      </section>
      {link && (
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="ghost" className="text-destructive hover:text-destructive">
              {t("page.disconnect")}
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("page.disconnectTitle", { repo: link.repo })}</AlertDialogTitle>
              <AlertDialogDescription>{t("page.disconnectBody")}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("page.cancel")}</AlertDialogCancel>
              <AlertDialogAction variant="destructive" onClick={() => unlink.mutate()}>
                {t("page.disconnect")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </div>
  );
}
