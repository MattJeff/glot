"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, Check, CircleAlert, Loader2, Rocket, RotateCcw } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useRouter } from "@/i18n/navigation";
import { api, ApiError, qk, type Project } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { EMPTY, MIN_N, draftPatch, loadDraft, saveDraft, useDebounced, type Draft } from "./draft";
import { StepAudience } from "./step-audience";
import { StepKeys } from "./step-keys";
import { StepLaunch } from "./step-launch";
import { StepVariants, useArms } from "./step-variants";

const STEPS = ["keys", "variants", "audience", "launch"] as const;

/** "home.hero.title" + "home.hero.cta" -> "home.hero" */
const commonPrefix = (keys: string[]) => {
  const parts = keys.map((k) => k.split("."));
  const out: string[] = [];
  for (let i = 0; parts.every((p) => i < p.length - 1 && p[i] === parts[0][i]); i++) out.push(parts[0][i]);
  return out.join(".") || keys[0] || "";
};

export function ExperimentWizard({ project }: { project: Project }) {
  const t = useTranslations("xpWizard");
  const errMsg = useErrorMessage();
  const router = useRouter();
  const qc = useQueryClient();
  const slug = project.slug;
  const locales = [project.source_locale, ...project.target_locales];
  const [initial] = useState(() => loadDraft(slug));
  const [d, setD] = useState<Draft>(initial ?? EMPTY);
  const [restored, setRestored] = useState(!!initial?.id);
  const [blocked, setBlocked] = useState<string | null>(null);
  const update = (patch: Partial<Draft>) => {
    setBlocked(null);
    setD((cur) => ({ ...cur, ...patch }));
  };
  const reset = () => {
    setRestored(false);
    setD(EMPTY);
    saveDraft(slug, null);
  };

  const s = useArms(slug, d.id, d.keys, locales);
  // The server draft disappeared or was launched elsewhere: start clean.
  const gone = (s.exp.error instanceof ApiError && s.exp.error.status === 404) || (s.exp.data && s.exp.data.status !== "draft");
  if (gone && d.id) reset(); // adjust state during render (no effect round-trip)

  // Auto-save: local mirror at once (resume after reload), server draft after a pause.
  const launched = useRef(false);
  useEffect(() => {
    if (!launched.current) saveDraft(slug, d);
  }, [slug, d]);
  const serverPatch = useDebounced(d.id ? JSON.stringify(draftPatch(d, t("audience.unnamed"))) : null, 800);
  const sync = useMutation({ mutationFn: (body: string) => api.xpUpdate(slug, d.id!, JSON.parse(body)) });
  const lastSynced = useRef<string | null>(null);
  useEffect(() => {
    if (!serverPatch || serverPatch === lastSynced.current || launched.current) return;
    lastSynced.current = serverPatch;
    sync.mutate(serverPatch);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverPatch]);

  const settings = useQuery({ queryKey: ["xp-settings", slug], queryFn: () => api.xpSettings(slug), retry: false });
  const settingsSeeded = useRef(false);
  useEffect(() => {
    if (settings.data && !settingsSeeded.current && !initial) {
      settingsSeeded.current = true;
      update({ persistence: settings.data.persistence });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.data]);

  // Why a step can't be left yet (null = ok). Shown on "Continue" in an alert, never as a silently disabled button.
  const badCountries = d.segments.some((sg) => (sg.rules.country ?? []).some((c) => !/^[A-Z]{2}$/.test(c)));
  const reason = (step: number): string | null => {
    if (step === 0 && d.keys.length === 0) return t("block.keys");
    if (step === 1) {
      if (s.arms.length === 0) return t("block.noArm");
      if (s.empty > 0) return t("block.empty", { n: s.empty });
      if (s.pending) return t("block.validating");
      if (s.errors > 0) return t("block.errors", { n: s.errors });
    }
    if (step === 2 && badCountries) return t("block.countries");
    if (step === 3) {
      if (!d.goal.trim()) return t("block.goal");
      if (d.revenue && !/^[A-Z]{3}$/.test(d.currency)) return t("block.currency");
      if (!(d.minN >= MIN_N)) return t("block.minN", { n: MIN_N });
    }
    return null;
  };
  const reachable = [0, 1, 2, 3].find((i) => reason(i) !== null) ?? 3;

  const heading = useRef<HTMLHeadingElement>(null);
  const alertRef = useRef<HTMLDivElement>(null);
  const go = (step: number) => {
    update({ step });
    requestAnimationFrame(() => heading.current?.focus());
  };

  // Step 1 -> 2 creates the server draft (or updates its keys).
  const ensure = useMutation({
    mutationFn: async () => {
      const name = d.name.trim() || commonPrefix(d.keys);
      if (d.id) return api.xpUpdate(slug, d.id, { ...draftPatch({ ...d, name }, t("audience.unnamed")) });
      return api.xpCreate(slug, { ...draftPatch({ ...d, name }, t("audience.unnamed")), locales });
    },
    onSuccess: (exp) => {
      qc.setQueryData(qk.xp(slug, exp.id), exp);
      update({ id: exp.id, name: exp.name, step: 1 });
      requestAnimationFrame(() => heading.current?.focus());
    },
  });

  const [problems, setProblems] = useState<string[]>([]);
  const launch = useMutation({
    mutationFn: async () => {
      await api.xpUpdate(slug, d.id!, draftPatch(d, t("audience.unnamed")));
      if (settings.data && settings.data.persistence !== d.persistence) await api.xpUpdateSettings(slug, { persistence: d.persistence });
      return api.xpStart(slug, d.id!);
    },
    onSuccess: (exp) => {
      launched.current = true;
      saveDraft(slug, null);
      qc.invalidateQueries({ queryKey: qk.variants(slug) });
      toast.success(t("launched"));
      router.push(`/app/${slug}/experiments/${exp.id}`);
    },
    onError: async (e) => {
      if (e instanceof ApiError && e.status === 422 && d.id) {
        const r = await api.xpCheck(slug, d.id).catch(() => null);
        setProblems(r?.problems ?? []);
      }
      requestAnimationFrame(() => alertRef.current?.focus());
    },
  });

  const next = (e: React.FormEvent) => {
    e.preventDefault();
    const why = reason(d.step);
    if (why) {
      setBlocked(why);
      requestAnimationFrame(() => alertRef.current?.focus());
      return;
    }
    if (d.step === 0) ensure.mutate();
    else if (d.step === 3) launch.mutate();
    else go(d.step + 1);
  };

  const step = STEPS[d.step] ?? "keys";
  const busy = ensure.isPending || launch.isPending;
  const error = blocked ?? (ensure.isError ? errMsg(ensure.error) : launch.isError ? errMsg(launch.error) : null);
  return (
    <div className="space-y-6">
      {restored && (
        <div role="status" className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border bg-muted/40 px-3 py-2 text-sm">
          <span className="text-muted-foreground">{t("restored")}</span>
          <Button type="button" variant="link" size="sm" className="h-auto p-0" onClick={reset}>
            <RotateCcw /> {t("startOver")}
          </Button>
        </div>
      )}

      <Stepper current={d.step} reachable={d.id ? reachable : 0} onGo={go} />

      <form onSubmit={next} noValidate aria-labelledby="xp-step-title" className="rounded-xl border bg-card">
        <div key={step} className="space-y-5 p-4 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-bottom-1 motion-safe:duration-200 sm:p-6">
          <header>
            <p className="text-xs font-medium text-muted-foreground">{t("stepOf", { n: d.step + 1, total: STEPS.length })}</p>
            <h2 id="xp-step-title" ref={heading} tabIndex={-1} className="mt-0.5 text-lg font-semibold tracking-tight outline-none">
              {t(`steps.${step}.title`)}
            </h2>
            <p className="mt-1 max-w-2xl text-sm text-pretty text-muted-foreground">{t(`steps.${step}.body`)}</p>
          </header>
          {step === "keys" && <StepKeys project={project} keys={d.keys} onChange={(keys) => update({ keys })} />}
          {step === "variants" && d.id && <StepVariants project={project} locales={locales} draft={d} update={update} arms={s} />}
          {step === "audience" && <StepAudience project={project} locales={locales} draft={d} update={update} armCount={s.arms.length} />}
          {step === "launch" && (
            <StepLaunch project={project} locales={locales} draft={d} update={update} armCount={s.arms.length} problems={problems} onEdit={go} />
          )}
        </div>

        <div className="sticky bottom-0 z-10 space-y-3 rounded-b-xl border-t bg-card p-4 sm:px-6">
          {error && (
            <div ref={alertRef} tabIndex={-1} role="alert" className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2.5 text-sm outline-none">
              <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
              <span>{error}</span>
            </div>
          )}
          <div className="flex items-center justify-between gap-3">
            {d.step > 0 ? (
              <Button type="button" variant="ghost" size="lg" onClick={() => go(d.step - 1)}>
                <ArrowLeft className="rtl:-scale-x-100" /> {t("back")}
              </Button>
            ) : (
              <span />
            )}
            <span className="hidden text-xs text-muted-foreground sm:inline" aria-hidden>
              {sync.isPending ? t("saving") : d.id ? t("autosaved") : ""}
            </span>
            <Button type="submit" size="lg" disabled={busy} aria-busy={busy}>
              {d.step === 3 ? (
                <>
                  {launch.isPending ? <Loader2 className="animate-spin" /> : <Rocket />} {launch.isPending ? t("launching") : t("launchCta")}
                </>
              ) : (
                <>
                  {ensure.isPending && <Loader2 className="animate-spin" />}
                  {t("continue", { step: t(`steps.${STEPS[d.step + 1]}.short`) })} <ArrowRight className="rtl:-scale-x-100" />
                </>
              )}
            </Button>
          </div>
        </div>
      </form>
    </div>
  );
}

function Stepper({ current, reachable, onGo }: { current: number; reachable: number; onGo: (i: number) => void }) {
  const t = useTranslations("xpWizard");
  return (
    <nav aria-label={t("progress")}>
      {/* Phones: one line + bar; the full list from sm up. */}
      <div className="sm:hidden">
        <div className="flex justify-between text-sm">
          <span className="font-medium">{t(`steps.${STEPS[current]}.short`)}</span>
          <span className="text-muted-foreground tabular-nums">{t("stepOf", { n: current + 1, total: STEPS.length })}</span>
        </div>
        <div className="mt-2 h-1 overflow-hidden rounded-full bg-muted">
          <div
            className="h-full origin-left rounded-full bg-brand transition-transform duration-300 ease-out rtl:origin-right"
            style={{ transform: `scaleX(${(current + 1) / STEPS.length})` }}
          />
        </div>
      </div>
      <ol className="hidden gap-2 sm:grid sm:grid-cols-4">
        {STEPS.map((s, i) => {
          const done = i < current;
          const can = i <= reachable && i !== current;
          return (
            <li key={s}>
              <button
                type="button"
                onClick={() => onGo(i)}
                disabled={!can}
                aria-current={i === current ? "step" : undefined}
                className={cn(
                  "flex min-h-11 w-full items-center gap-2.5 rounded-lg border px-3 py-2 text-start text-sm transition-colors duration-150 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                  i === current ? "border-brand bg-brand/5 font-medium" : "hover:bg-muted disabled:hover:bg-transparent",
                  !can && i !== current && "cursor-default text-muted-foreground",
                )}
              >
                <span
                  aria-hidden
                  className={cn(
                    "grid size-6 shrink-0 place-items-center rounded-full border text-xs tabular-nums",
                    done && "border-brand bg-brand text-brand-foreground",
                    i === current && "border-brand text-brand",
                  )}
                >
                  {done ? <Check className="size-3.5" /> : i + 1}
                </span>
                <span className="min-w-0 truncate">{t(`steps.${s}.short`)}</span>
                {done && <span className="sr-only">{t("done")}</span>}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
