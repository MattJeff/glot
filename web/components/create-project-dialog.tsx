"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useRef, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { LocalePicker } from "@/components/onboarding/locale-picker";
import { browserLocale } from "@/components/onboarding/locales";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useRouter } from "@/i18n/navigation";
import { api, ApiError, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";

export const slugify = (s: string) =>
  s
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48);

export const parseLocales = (s: string) =>
  [...new Set(s.split(/[\s,]+/).map((l) => l.trim()).filter(Boolean))];

const noop = () => () => {};
const STEP_IDS = ["name", "source", "targets"] as const;
const STEPS = STEP_IDS.length;

/** name → source language (from the browser) → target languages → create, then the onboarding checklist. */
export function CreateProjectDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const t = useTranslations("projects.create");
  const tc = useTranslations("common");
  const message = useErrorMessage();
  const qc = useQueryClient();
  const router = useRouter();
  const detected = useSyncExternalStore(noop, browserLocale, () => "en");
  const [step, setStep] = useState(0);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);
  const [source, setSource] = useState<string | null>(null);
  const [targets, setTargets] = useState<string[]>([]);
  const body = useRef<HTMLDivElement>(null);
  const src = source ?? detected;
  const slugValid = /^[a-z0-9][a-z0-9-]*[a-z0-9]$/.test(slug);

  const reset = () => {
    setStep(0);
    setName("");
    setSlug("");
    setSlugTouched(false);
    setSource(null);
    setTargets([]);
  };
  // Focus the first field of each step, so keyboard and screen reader users follow the flow.
  const go = (n: number) => {
    setStep(n);
    requestAnimationFrame(() => body.current?.querySelector<HTMLElement>("input, button")?.focus());
  };
  const create = useMutation({
    mutationFn: api.createProject,
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: qk.projects });
      toast.success(t("success", { name: p.name }));
      onOpenChange(false);
      reset();
      router.push(`/app/${p.slug}/onboarding`);
    },
    onError: (e) => {
      if (e instanceof ApiError && e.code === "slug_taken") go(0);
    },
  });
  const slugTaken = create.error instanceof ApiError && create.error.code === "slug_taken";


  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        onOpenChange(v);
        if (!v) create.reset();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (step < STEPS - 1) return go(step + 1);
            create.mutate({ name: name.trim(), slug, source_locale: src, target_locales: targets.filter((l) => l !== src) });
          }}
          className="space-y-5"
        >
          <DialogHeader>
            <p className="text-xs font-medium text-muted-foreground tabular-nums">{t("step", { n: step + 1, total: STEPS })}</p>
            <DialogTitle>{t(`steps.${STEP_IDS[step]}.title`)}</DialogTitle>
            <DialogDescription>{t(`steps.${STEP_IDS[step]}.description`)}</DialogDescription>
            <div className="flex gap-1.5 pt-1" aria-hidden>
              {Array.from({ length: STEPS }, (_, i) => (
                <span
                  key={i}
                  className={cn("h-1 flex-1 rounded-full bg-muted transition-colors duration-300", i <= step && "bg-brand")}
                />
              ))}
            </div>
          </DialogHeader>

          <div ref={body} key={step} className="min-h-56 animate-in fade-in-0 slide-in-from-bottom-1 duration-200 motion-reduce:animate-none">
            {step === 0 && (
              <div className="space-y-4">
                <div className="space-y-1.5">
                  <Label htmlFor="p-name">{t("name")}</Label>
                  <Input
                    id="p-name"
                    required
                    maxLength={100}
                    value={name}
                    placeholder={t("namePlaceholder")}
                    onChange={(e) => {
                      setName(e.target.value);
                      if (!slugTouched) setSlug(slugify(e.target.value));
                    }}
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="p-slug">{t("slug")}</Label>
                  <Input
                    id="p-slug"
                    required
                    dir="ltr"
                    pattern="[a-z0-9][a-z0-9\-]*[a-z0-9]"
                    minLength={2}
                    maxLength={63}
                    className="font-mono"
                    value={slug}
                    aria-invalid={slugTaken || undefined}
                    aria-describedby="p-slug-hint"
                    onChange={(e) => {
                      setSlugTouched(true);
                      setSlug(e.target.value);
                      create.reset();
                    }}
                  />
                  <p id="p-slug-hint" className={cn("text-xs", slugTaken ? "text-destructive" : "text-muted-foreground")}>
                    {slugTaken ? t("slugTaken") : t("slugHint")}
                  </p>
                </div>
              </div>
            )}
            {step === 1 && (
              <LocalePicker
                mode="single"
                label={t("sourceLocale")}
                value={[src]}
                onChange={([l]) => {
                  setSource(l);
                  setTargets((ts) => ts.filter((x) => x !== l));
                }}
                detected={detected}
              />
            )}
            {step === 2 && (
              <LocalePicker mode="multi" label={t("targetLocales")} value={targets} onChange={setTargets} exclude={src} />
            )}
          </div>

          {create.isError && !slugTaken && (
            <p role="alert" className="text-sm text-destructive">
              {message(create.error)}
            </p>
          )}

          <DialogFooter className="gap-2 sm:justify-between">
            {step > 0 ? (
              <Button type="button" variant="ghost" onClick={() => go(step - 1)}>
                <ArrowLeft aria-hidden className="rtl:-scale-x-100" />
                {t("back")}
              </Button>
            ) : (
              <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
                {tc("cancel")}
              </Button>
            )}
            {step < STEPS - 1 ? (
              <Button type="submit" disabled={step === 0 && (!name.trim() || !slugValid)}>
                {t("next")}
                <ArrowRight aria-hidden className="rtl:-scale-x-100" />
              </Button>
            ) : (
              <Button type="submit" disabled={create.isPending || targets.length === 0} aria-busy={create.isPending}>
                {create.isPending && <Loader2 aria-hidden className="animate-spin" />}
                {targets.length === 0 ? t("pickOne") : t("submit", { count: targets.length })}
              </Button>
            )}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
