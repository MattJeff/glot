"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useParams } from "next/navigation";
import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { useRouter } from "@/i18n/navigation";
import { api, qk, type Job } from "@/lib/api";

export function useSlug() {
  const params = useParams<{ slug?: string }>();
  return params.slug ? decodeURIComponent(params.slug) : "";
}

export function useProject(slug: string) {
  return useQuery({ queryKey: qk.project(slug), queryFn: () => api.project(slug), enabled: !!slug });
}

export function useTranslateMissing(slug: string) {
  const t = useTranslations("overview");
  const qc = useQueryClient();
  const router = useRouter();
  return useMutation({
    mutationFn: () => api.createJob(slug, { kind: "translate" }),
    onSuccess: (job) => {
      qc.setQueryData<Job[]>(qk.jobs(slug), (jobs) => [job, ...(jobs ?? [])]);
      toast.success(t("jobStarted"));
      router.push(`/app/${slug}/jobs`);
    },
  });
}

const noop = () => () => {};
/** window.location.origin without a hydration mismatch (undefined during SSR). */
export function useOrigin() {
  return useSyncExternalStore(noop, () => window.location.origin, () => undefined);
}
