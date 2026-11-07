"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Loader2, Rocket } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { useSigned, VerdictBadge } from "@/components/launch-story";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, qk, type NewLaunch } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";

export default function LaunchesPage() {
  const t = useTranslations("launches");
  const format = useFormatter();
  const name = useLocaleName();
  const signed = useSigned();
  const slug = useSlug();
  const [open, setOpen] = useState(false);
  const q = useQuery({ queryKey: qk.launches(slug), queryFn: () => api.launches(slug) });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="mx-auto max-w-5xl">
      <PageHeader
        eyebrow={
          <Link href={`/app/${slug}/insights`} className="inline-flex items-center gap-1 hover:text-foreground">
            <ArrowLeft className="size-3 rtl:rotate-180" /> {t("back")}
          </Link>
        }
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button onClick={() => setOpen(true)}>
            <Rocket /> {t("new")}
          </Button>
        }
      />
      {!q.data ? (
        <Skeleton className="h-32 rounded-xl" />
      ) : q.data.length === 0 ? (
        <p className="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">{t("empty")}</p>
      ) : (
        <ul className="grid gap-3 md:grid-cols-2">
          {q.data.map((l) => (
            <li key={l.id}>
              <Link href={`/app/${slug}/insights/launches/${l.id}`} className="block rounded-xl border p-4 transition-colors hover:bg-muted/40">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <h2 className="truncate font-medium">{name(l.locale)}</h2>
                      <span className="font-mono text-xs text-muted-foreground">{l.locale}</span>
                    </div>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {t(`mode.${l.mode}`)} · {t(`status.${l.status}`)} · {format.dateTime(new Date(l.launched_at), { day: "numeric", month: "short" })}
                    </p>
                  </div>
                  <VerdictBadge verdict={l.verdict} />
                </div>
                <div className="mt-3 text-sm tabular-nums">
                  <b className="font-semibold">{signed(l.lift)}</b>{" "}
                  <span className="text-xs text-muted-foreground">
                    [{signed(l.lift_lo)} ; {signed(l.lift_hi)}] · {l.goal}
                  </span>
                </div>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {open && <NewLaunchDialog slug={slug} onClose={() => setOpen(false)} />}
    </div>
  );
}

function NewLaunchDialog({ slug, onClose }: { slug: string; onClose: () => void }) {
  const t = useTranslations("launches");
  const name = useLocaleName();
  const qc = useQueryClient();
  const project = useProject(slug);
  const p = project.data;
  const [b, setB] = useState<NewLaunch>({ locale: p?.target_locales[0] ?? "", mode: "holdout", holdout_pct: 20, days: 14, goal: "signup" });
  const set = (patch: Partial<NewLaunch>) => setB((x) => ({ ...x, ...patch }));
  // Guardrail events, comma-separated: [a drop is bad, a rise is bad].
  const [guards, setGuards] = useState(["", ""]);
  const events = (s: string) => s.split(",").map((e) => e.trim()).filter(Boolean);
  const holdout = b.mode === "holdout";
  const create = useMutation({
    mutationFn: () =>
      api.createLaunch(slug, {
        ...b,
        locale: b.locale.trim(),
        holdout_pct: holdout ? b.holdout_pct : undefined,
        auto_rollout: holdout && !!b.auto_rollout,
        guardrails: guards.flatMap((g, i) => events(g).map((event) => ({ event, higher_is_worse: i === 1 }))),
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.launches(slug) });
      qc.invalidateQueries({ queryKey: qk.project(slug) });
      toast.success(t("started", { language: name(b.locale.trim()) }));
      onClose();
    },
  });

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("dialog.title")}</DialogTitle>
          <DialogDescription>{t("dialog.body")}</DialogDescription>
        </DialogHeader>
        <form
          id="launch-form"
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="launch-locale">{t("dialog.locale")}</Label>
              <Input id="launch-locale" value={b.locale} onChange={(e) => set({ locale: e.target.value })} placeholder="es" translate="no" required className="font-mono" />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="launch-mode">{t("dialog.mode")}</Label>
              <Select value={b.mode} onValueChange={(v) => set({ mode: v as NewLaunch["mode"] })}>
                <SelectTrigger id="launch-mode" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="holdout">{t("mode.holdout")}</SelectItem>
                  <SelectItem value="before_after">{t("mode.before_after")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <p className="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">
            {b.mode === "holdout" ? t("mode.holdoutHint", { fallback: name(p?.source_locale ?? "en") }) : t("mode.before_afterHint")}
          </p>
          <div className="grid grid-cols-3 gap-3">
            {b.mode === "holdout" && (
              <div className="grid gap-1.5">
                <Label htmlFor="launch-pct">{t("dialog.holdout")}</Label>
                <Input id="launch-pct" type="number" min={5} max={50} value={b.holdout_pct} onChange={(e) => set({ holdout_pct: Number(e.target.value) })} />
              </div>
            )}
            <div className="grid gap-1.5">
              <Label htmlFor="launch-days">{t("dialog.days")}</Label>
              <Input id="launch-days" type="number" min={7} max={90} value={b.days} onChange={(e) => set({ days: Number(e.target.value) })} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="launch-goal">{t("dialog.goal")}</Label>
              <Input id="launch-goal" value={b.goal} onChange={(e) => set({ goal: e.target.value })} className="font-mono" />
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            {[t("dialog.guardDown"), t("dialog.guardUp")].map((label, i) => (
              <div key={label} className="grid gap-1.5">
                <Label htmlFor={`launch-guard-${i}`}>{label}</Label>
                <Input
                  id={`launch-guard-${i}`}
                  value={guards[i]}
                  onChange={(e) => setGuards((g) => g.map((x, j) => (j === i ? e.target.value : x)))}
                  placeholder={i ? "error" : "purchase"}
                  className="font-mono"
                />
              </div>
            ))}
          </div>
          <p className="-mt-2 text-xs text-muted-foreground">{t("dialog.guardHint")}</p>
          {holdout && (
            <div className="flex items-start gap-2">
              <Checkbox id="launch-auto" checked={!!b.auto_rollout} onCheckedChange={(v) => set({ auto_rollout: v === true })} className="mt-0.5" />
              <Label htmlFor="launch-auto" className="font-normal leading-snug">
                {t("dialog.auto")}
              </Label>
            </div>
          )}
        </form>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("dialog.cancel")}
          </Button>
          <Button type="submit" form="launch-form" disabled={create.isPending || !b.locale.trim()}>
            {create.isPending ? <Loader2 className="animate-spin" /> : <Rocket />} {t("dialog.start")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
