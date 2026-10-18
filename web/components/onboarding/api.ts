"use client";

// Client for crates/server/src/onboarding.rs. Kept apart from lib/api.ts (shared by every agent) on purpose.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError } from "@/lib/api";

export type OnboardingPath = "agent" | "terminal" | "github";
export type StepId =
  | "path"
  | "token"
  | "first_push"
  | "first_translation"
  | "zero_errors"
  | "invite"
  | "glossary"
  | "cdn"
  | "badge";
export type AchievementId = "first_push" | "first_locale_100" | "zero_errors" | "five_languages" | "glossary_set";
export const ACHIEVEMENTS: AchievementId[] = ["first_push", "first_locale_100", "zero_errors", "five_languages", "glossary_set"];
export type LocaleProgress = { locale: string; translated: number; reviewed: number; missing: number; errors: number };
export type Onboarding = {
  server_url: string;
  slug: string;
  source_locale: string;
  keys: number;
  first_push_at: string | null;
  locales: LocaleProgress[];
  path: OnboardingPath | null;
  hidden: boolean;
  steps: { id: StepId; done: boolean }[];
  achievements: { id: AchievementId; unlocked_at: string }[];
};

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  }).catch(() => {
    throw new ApiError(0, "network", "Network error");
  });
  if (!res.ok) {
    const err = (await res.json().catch(() => null)) as { error?: { code: string; message: string } } | null;
    throw new ApiError(res.status, err?.error?.code ?? "http_error", err?.error?.message ?? res.statusText);
  }
  return (res.status === 204 ? undefined : await res.json()) as T;
}

const url = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/onboarding`;
export const onboardingKey = (slug: string) => ["onboarding", slug] as const;

/** Polls every 2 s until the first push lands (the live "waiting" state), then every 10 s if `live`. */
export function useOnboarding(slug: string, live = false) {
  return useQuery({
    queryKey: onboardingKey(slug),
    queryFn: () => call<Onboarding>("GET", url(slug)),
    enabled: !!slug,
    refetchInterval: (q) => (!live ? false : (q.state.data?.keys ?? 0) === 0 ? 2000 : 10000),
  });
}

export function usePatchOnboarding(slug: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (b: { path?: OnboardingPath; hidden?: boolean }) => call<void>("PATCH", url(slug), b),
    onMutate: (b) => {
      qc.setQueryData<Onboarding>(onboardingKey(slug), (o) => o && { ...o, ...b });
    },
    onSettled: () => qc.invalidateQueries({ queryKey: onboardingKey(slug) }),
  });
}

export function useCreateOnboardingToken(slug: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => call<{ token: string }>("POST", `${url(slug)}/token`),
    onSuccess: () => qc.invalidateQueries({ queryKey: onboardingKey(slug) }),
  });
}

/** The single most useful thing to do next, from real project state. */
export type NextAction =
  | { kind: "push" }
  | { kind: "translate"; locale: string; count: number }
  | { kind: "fix"; locale: string; count: number }
  | { kind: "glossary" }
  | { kind: "cdn" };

export function nextAction(o: Onboarding): NextAction | null {
  if (o.keys === 0) return { kind: "push" };
  const most = (f: (l: LocaleProgress) => number) =>
    o.locales.reduce<LocaleProgress | null>((best, l) => (f(l) > 0 && (!best || f(l) > f(best)) ? l : best), null);
  const missing = most((l) => l.missing);
  if (missing) return { kind: "translate", locale: missing.locale, count: missing.missing };
  const errors = most((l) => l.errors);
  if (errors) return { kind: "fix", locale: errors.locale, count: errors.errors };
  const done = (id: StepId) => o.steps.find((s) => s.id === id)?.done;
  if (!done("glossary")) return { kind: "glossary" };
  if (!done("cdn")) return { kind: "cdn" };
  return null;
}
