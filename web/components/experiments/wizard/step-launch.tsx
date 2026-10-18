"use client";

import { useQuery } from "@tanstack/react-query";
import { Cookie, ShieldCheck } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { api, qk, type Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { MIN_N, type Draft } from "./draft";

const CURRENCIES = ["EUR", "USD", "GBP", "CHF", "JPY", "CAD", "AUD"];

export function StepLaunch({
  project,
  locales,
  draft: d,
  update,
  armCount,
  problems,
  onEdit,
}: {
  project: Project;
  locales: string[];
  draft: Draft;
  update: (p: Partial<Draft>) => void;
  armCount: number;
  problems: string[];
  onEdit: (step: number) => void;
}) {
  const t = useTranslations("xpWizard.launch");
  const format = useFormatter();
  const localeName = useLocaleName();
  // Events your site already sent in the last 30 days (glot.track).
  const insights = useQuery({ queryKey: [...qk.insights(project.slug), 30], queryFn: () => api.insights(project.slug, { days: 30 }), retry: false });
  const events = insights.data?.events ?? [];

  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_20rem]">
      <div className="min-w-0 space-y-6">
        <div className="grid gap-1.5">
          <Label htmlFor="xp-name">{t("name")}</Label>
          <Input id="xp-name" value={d.name} onChange={(e) => update({ name: e.target.value })} className="h-9" maxLength={120} />
        </div>

        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">{t("goal")}</legend>
          <p className="text-xs text-muted-foreground" id="xp-goal-hint">
            {events.length ? t("goalHint") : t("goalHintNone")}
          </p>
          {events.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {events.map((e) => (
                <button
                  key={e}
                  type="button"
                  aria-pressed={d.goal === e}
                  onClick={() => update({ goal: e })}
                  className={cn(
                    "min-h-9 rounded-full border px-3 font-mono text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                    d.goal === e && "border-brand bg-brand/10 font-medium",
                  )}
                >
                  {e}
                </button>
              ))}
            </div>
          )}
          <Label htmlFor="xp-goal" className="sr-only">
            {t("goalInput")}
          </Label>
          <Input
            id="xp-goal"
            value={d.goal}
            onChange={(e) => update({ goal: e.target.value })}
            placeholder="signup"
            className="h-9 max-w-xs font-mono"
            autoComplete="off"
            translate="no"
            aria-describedby="xp-goal-hint"
          />
        </fieldset>

        <div className="space-y-3 rounded-lg border p-3 sm:p-4">
          <div className="flex items-start justify-between gap-4">
            <div>
              <Label htmlFor="xp-revenue">{t("revenue")}</Label>
              <p className="mt-0.5 text-xs text-pretty text-muted-foreground">{t("revenueHint")}</p>
            </div>
            <Switch id="xp-revenue" checked={d.revenue} onCheckedChange={(revenue) => update({ revenue })} />
          </div>
          {d.revenue && (
            <div className="grid max-w-40 gap-1.5">
              <Label htmlFor="xp-currency">{t("currency")}</Label>
              <Input id="xp-currency" list="xp-currencies" value={d.currency} onChange={(e) => update({ currency: e.target.value.toUpperCase().slice(0, 3) })} className="h-9 font-mono" translate="no" />
              <datalist id="xp-currencies">
                {CURRENCIES.map((c) => (
                  <option key={c} value={c} />
                ))}
              </datalist>
            </div>
          )}
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="xp-min">{t("minN")}</Label>
          <Input
            id="xp-min"
            type="number"
            inputMode="numeric"
            min={MIN_N}
            step={50}
            value={d.minN}
            onChange={(e) => update({ minN: Number(e.target.value) })}
            className="h-9 max-w-32 tabular-nums"
            aria-describedby="xp-min-hint"
          />
          <p id="xp-min-hint" className="text-xs text-pretty text-muted-foreground">
            {t("minNHint", { n: MIN_N })}
          </p>
        </div>

        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">{t("cookie")}</legend>
          <div className="grid gap-2 sm:grid-cols-2">
            {(["none", "session"] as const).map((m) => (
              <label
                key={m}
                className={cn(
                  "flex cursor-pointer gap-3 rounded-lg border p-3 transition-colors duration-150 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-background hover:bg-muted/50",
                  d.persistence === m && "border-brand bg-brand/5",
                )}
              >
                <input type="radio" name="xp-cookie" value={m} checked={d.persistence === m} onChange={() => update({ persistence: m })} className="mt-1 size-4 accent-(--brand)" />
                <span>
                  <span className="flex items-center gap-1.5 text-sm font-medium">
                    {m === "session" ? <Cookie className="size-4" aria-hidden /> : <ShieldCheck className="size-4" aria-hidden />}
                    {t(`cookieMode.${m}.label`)}
                  </span>
                  <span className="mt-0.5 block text-xs text-pretty text-muted-foreground">{t(`cookieMode.${m}.body`)}</span>
                </span>
              </label>
            ))}
          </div>
          <p className="text-xs text-pretty text-muted-foreground">{t("gdpr")}</p>
        </fieldset>
      </div>

      <aside aria-labelledby="xp-recap" className="h-fit space-y-3 rounded-lg border bg-muted/30 p-4 lg:sticky lg:top-4">
        <h3 id="xp-recap" className="text-sm font-medium">
          {t("recap")}
        </h3>
        <dl className="space-y-3 text-sm">
          <Row label={t("recapKeys")} onEdit={() => onEdit(0)} editLabel={t("edit", { what: t("recapKeys") })}>
            <span className="font-mono text-xs break-all" translate="no">
              {d.keys.join(", ")}
            </span>
          </Row>
          <Row label={t("recapVariants")} onEdit={() => onEdit(1)} editLabel={t("edit", { what: t("recapVariants") })}>
            {t("recapVariantsValue", { n: armCount, locales: locales.length })}
            <span className="block text-xs text-muted-foreground">{locales.map((l) => localeName(l)).join(", ")}</span>
          </Row>
          <Row label={t("recapAudience")} onEdit={() => onEdit(2)} editLabel={t("edit", { what: t("recapAudience") })}>
            {d.segments.length ? d.segments.map((s, i) => s.name || t("segmentN", { n: i + 1 })).join(", ") : t("everyone")}
          </Row>
          <Row label={t("recapGoal")}>
            {d.goal ? <span className="font-mono">{d.goal}</span> : <span className="text-muted-foreground">{t("notSet")}</span>}
            {d.revenue && <span className="text-muted-foreground"> · {t("revenueIn", { currency: d.currency })}</span>}
            <span className="block text-xs text-muted-foreground">{t("recapMin", { n: format.number(d.minN || MIN_N) })}</span>
          </Row>
          <Row label={t("cookie")}>{t(`cookieMode.${d.persistence}.label`)}</Row>
        </dl>
        {problems.length > 0 && (
          <div className="border-t pt-3">
            <p className="text-xs font-medium text-destructive">{t("problems")}</p>
            <ul className="mt-1 list-disc space-y-0.5 ps-4 text-xs">
              {problems.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          </div>
        )}
      </aside>
    </div>
  );
}

function Row({ label, children, onEdit, editLabel }: { label: string; children: React.ReactNode; onEdit?: () => void; editLabel?: string }) {
  const t = useTranslations("xpWizard.launch");
  return (
    <div>
      <dt className="flex items-center justify-between text-xs text-muted-foreground">
        {label}
        {onEdit && (
          <Button type="button" variant="link" size="sm" className="h-6 px-0 text-xs" onClick={onEdit} aria-label={editLabel}>
            {t("editShort")}
          </Button>
        )}
      </dt>
      <dd className="mt-0.5">{children}</dd>
    </div>
  );
}
