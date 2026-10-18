"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FlaskConical, Loader2, Sparkles, Trophy, Users } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { api, qk, type VariantExperiment } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";
import { ExperimentCard } from "./parts";

// Integration snippet shown as code, not UI copy.
const SNIPPET = `// app/[locale]/layout.tsx — inside NextIntlClientProvider
import { GlotVariants } from "@glot/runtime/next-intl";
<GlotVariants>{children}</GlotVariants>

// anywhere: the conversion that decides the winner
import { track } from "@glot/insights";
track("signup");`;

export default function ExperimentsPage() {
  const t = useTranslations("experiments");
  const format = useFormatter();
  const name = useLocaleName();
  const slug = useSlug();
  const qc = useQueryClient();
  const project = useProject(slug);
  const [open, setOpen] = useState(false);
  const q = useQuery({ queryKey: qk.variants(slug), queryFn: () => api.variantExperiments(slug), refetchInterval: 30_000 });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "promote" | "stop" }) => api.variantAction(slug, id, action),
    onSuccess: (_, v) => {
      qc.invalidateQueries({ queryKey: qk.variants(slug) });
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      toast.success(t(v.action === "promote" ? "promotedToast" : "stoppedToast"));
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const all = q.data;
  const live = all?.filter((e) => e.status === "running" || e.status === "decided") ?? [];
  const past = all?.filter((e) => e.status === "promoted" || e.status === "stopped") ?? [];
  const byLocale = Object.entries(
    live.reduce<Record<string, VariantExperiment[]>>((m, e) => ((m[e.locale] ??= []).push(e), m), {}),
  ).sort(([a], [b]) => a.localeCompare(b));
  const toPromote = live.filter((e) => e.status === "decided" && e.winner !== 0).length;
  const visitors = live.reduce((n, e) => n + e.arms.reduce((s, a) => s + a.views, 0), 0);

  const card = (e: VariantExperiment) => (
    <ExperimentCard
      key={e.id}
      exp={e}
      busy={act.isPending && act.variables?.id === e.id}
      onAction={(action) => act.mutate({ id: e.id, action })}
    />
  );

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button onClick={() => setOpen(true)} disabled={!project.data}>
            <FlaskConical /> {t("new")}
          </Button>
        }
      />

      {!all ? (
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-24 rounded-xl" />
            ))}
          </div>
          <Skeleton className="h-96 rounded-xl" />
        </div>
      ) : all.length === 0 ? (
        <>
          <div className="flex flex-col items-center rounded-xl border border-dashed px-6 py-10 text-center">
            <div className="mb-3 grid size-10 place-items-center rounded-lg border bg-muted/50">
              <Sparkles className="size-5 text-muted-foreground" />
            </div>
            <h2 className="font-medium">{t("empty.title")}</h2>
            <p className="mt-1 max-w-lg text-sm text-muted-foreground">{t("empty.body")}</p>
            <Button className="mt-4" onClick={() => setOpen(true)} disabled={!project.data}>
              <FlaskConical /> {t("new")}
            </Button>
          </div>
          <CodeBlock code={SNIPPET} label={t("install")} />
        </>
      ) : (
        <>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <Stat icon={FlaskConical} label={t("stats.live")} value={format.number(live.length)} hint={t("stats.liveHint", { n: byLocale.length })} />
            <Stat icon={Trophy} label={t("stats.toPromote")} value={format.number(toPromote)} hint={t("stats.toPromoteHint")} />
            <Stat icon={Users} label={t("stats.visitors")} value={format.number(visitors)} hint={t("stats.visitorsHint")} />
          </div>

          {byLocale.map(([locale, exps]) => (
            <section key={locale} aria-labelledby={`loc-${locale}`} className="space-y-3">
              <h2 id={`loc-${locale}`} className="flex items-center gap-2 font-medium">
                {name(locale)} <span className="font-mono text-xs text-muted-foreground">{locale}</span>
              </h2>
              {exps.map(card)}
            </section>
          ))}

          {past.length > 0 && (
            <details className="group rounded-xl border p-4 sm:p-5">
              <summary className="cursor-pointer font-medium">{t("history", { n: past.length })}</summary>
              <div className="mt-4 space-y-3">{past.map(card)}</div>
            </details>
          )}
          <CodeBlock code={SNIPPET} label={t("install")} />
        </>
      )}

      {project.data && (
        <NewExperimentDialog
          key={String(open)}
          open={open}
          slug={slug}
          locales={project.data.target_locales}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}

function NewExperimentDialog({ open, slug, locales, onClose }: { open: boolean; slug: string; locales: string[]; onClose: () => void }) {
  const t = useTranslations("experiments.dialog");
  const name = useLocaleName();
  const qc = useQueryClient();
  const [key, setKey] = useState("");
  const [locale, setLocale] = useState(locales[0] ?? "");
  const [variants, setVariants] = useState("3");
  const [explore, setExplore] = useState(100);
  const [minN, setMinN] = useState(500);
  const [goal, setGoal] = useState("signup");
  // Suggestions for the key field (native datalist): the first page of keys matching what was typed.
  const keys = useQuery({
    queryKey: [...qk.keys(slug), "suggest", key],
    queryFn: () => api.keys(slug, { q: key || undefined, per_page: 20 }),
    enabled: open,
  });
  const create = useMutation({
    mutationFn: () =>
      api.createVariantExperiment(slug, {
        key: key.trim(),
        locale,
        variants: Number(variants),
        explore_pct: explore,
        min_n: minN,
        goal: goal.trim() || undefined,
      }),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: qk.variants(slug) });
      toast.success(t("created", { n: r.arms.length - 1, rejected: r.rejected.length }));
      onClose();
    },
  });

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("title")}</DialogTitle>
          <DialogDescription>{t("body")}</DialogDescription>
        </DialogHeader>
        <form
          id="variant-form"
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="v-key">{t("key")}</Label>
            <Input
              id="v-key"
              list="v-keys"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder="pricing.cta"
              translate="no"
              required
              className="font-mono"
              autoComplete="off"
            />
            <datalist id="v-keys">
              {keys.data?.items.map((k) => (
                <option key={k.key} value={k.key}>
                  {k.source}
                </option>
              ))}
            </datalist>
            <p className="text-xs text-muted-foreground">{t("keyHint")}</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="v-locale">{t("locale")}</Label>
              <Select value={locale} onValueChange={setLocale}>
                <SelectTrigger id="v-locale" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {locales.map((l) => (
                    <SelectItem key={l} value={l}>
                      {name(l)} <span className="font-mono text-xs text-muted-foreground">{l}</span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="v-count">{t("variants")}</Label>
              <Select value={variants} onValueChange={setVariants}>
                <SelectTrigger id="v-count" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {["2", "3"].map((n) => (
                    <SelectItem key={n} value={n}>
                      {n}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="v-explore">{t("explore")}</Label>
              <Input id="v-explore" type="number" min={1} max={100} value={explore} onChange={(e) => setExplore(Number(e.target.value))} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="v-min">{t("minN")}</Label>
              <Input id="v-min" type="number" min={50} step={50} value={minN} onChange={(e) => setMinN(Number(e.target.value))} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="v-goal">{t("goal")}</Label>
              <Input id="v-goal" value={goal} onChange={(e) => setGoal(e.target.value)} className="font-mono" />
            </div>
          </div>
          <p className="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">{t("guardrails")}</p>
        </form>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button type="submit" form="variant-form" disabled={create.isPending || !key.trim() || !locale}>
            {create.isPending ? <Loader2 className="animate-spin" /> : <Sparkles />} {t("submit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Stat({ icon: Icon, label, value, hint }: { icon: React.ComponentType<{ className?: string }>; label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border p-4">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="size-3.5" /> {label}
      </div>
      <div className="mt-1.5 text-2xl font-semibold tracking-tight tabular-nums">{value}</div>
      {hint && <div className="mt-1 truncate text-xs text-muted-foreground">{hint}</div>}
    </div>
  );
}
