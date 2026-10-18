"use client";

import { useQuery } from "@tanstack/react-query";
import { Globe2, Plus, Trash2, TriangleAlert, Users } from "lucide-react";
import { useFormatter, useLocale, useTranslations } from "next-intl";
import { useState } from "react";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, type Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { ATTRS, MAX_SEGMENTS, segmentsToCells, uid, useDebounced, type Draft, type Segment, type SegmentAttr } from "./draft";

const TOO_LONG = 90; // days: past this a cell is merged into "everyone else" (design doc §4)
const PRESETS = [
  { name: "mobile", rules: { device: ["mobile"] } },
  { name: "search", rules: { source: ["search"] } },
  { name: "new", rules: { visitor: ["new"] } },
] as const satisfies { name: string; rules: Segment["rules"] }[];

export function StepAudience({
  project,
  locales,
  draft: d,
  update,
  armCount,
}: {
  project: Project;
  locales: string[];
  draft: Draft;
  update: (p: Partial<Draft>) => void;
  armCount: number;
}) {
  const t = useTranslations("xpWizard.audience");
  const format = useFormatter();
  const setSegments = (segments: Segment[]) => update({ segments });
  const patch = (id: string, p: Partial<Segment>) => setSegments(d.segments.map((s) => (s.id === id ? { ...s, ...p } : s)));
  const add = (preset?: (typeof PRESETS)[number]) =>
    setSegments([...d.segments, { id: uid(), name: preset ? t(`preset.${preset.name}`) : "", rules: preset ? { ...preset.rules } : {} }]);

  const cells = useDebounced(segmentsToCells(d.segments, t("unnamed")), 500);
  const estimate = useQuery({
    queryKey: ["xp-estimate", project.slug, cells, armCount, d.goal, d.minN],
    queryFn: () => api.xpEstimate(project.slug, { cells, arms: armCount + 1, locales, goal: d.goal || undefined, min_n: d.minN }),
    retry: false,
    staleTime: 60_000,
  });
  // Estimate rows are matched by position: targeted segments first, then "*".
  const est = (i: number) => estimate.data?.cells[i];
  const rest = estimate.data?.cells.find((c) => c.cell === "*") ?? estimate.data?.cells[cells.length];
  const days = (n: number | null | undefined) => (n == null ? "—" : t("days", { n: Math.ceil(n) }));
  const slow = estimate.data?.cells.filter((c) => c.too_small || (c.days ?? 0) > TOO_LONG) ?? [];
  const fragmented = cells.length >= 3 || slow.length > 0;

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="outline" size="lg" onClick={() => add()} disabled={d.segments.length >= MAX_SEGMENTS}>
          <Plus /> {t("add")}
        </Button>
        {PRESETS.map((p) => (
          <Button key={p.name} type="button" variant="ghost" size="lg" onClick={() => add(p)} disabled={d.segments.length >= MAX_SEGMENTS}>
            <Plus /> {t(`preset.${p.name}`)}
          </Button>
        ))}
        {d.segments.length >= MAX_SEGMENTS && <span className="text-xs text-muted-foreground">{t("max", { n: MAX_SEGMENTS })}</span>}
      </div>

      <ul className="space-y-3">
        {d.segments.map((s, i) => (
          <li key={s.id} className="rounded-lg border p-3 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-bottom-1 motion-safe:duration-200 sm:p-4">
            <SegmentEditor
              segment={s}
              index={i}
              locales={locales}
              onChange={(p) => patch(s.id, p)}
              onRemove={() => setSegments(d.segments.filter((x) => x.id !== s.id))}
            />
            <Estimate row={est(i)} loading={estimate.isPending} format={format} days={days} />
          </li>
        ))}
        <li className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-dashed p-3 sm:p-4">
          <Users className="size-4 text-muted-foreground" aria-hidden />
          <span className="font-medium">{d.segments.length ? t("everyoneElse") : t("everyone")}</span>
          <span className="text-sm text-muted-foreground">{d.segments.length ? t("everyoneElseHint") : t("everyoneHint")}</span>
          <div className="basis-full">
            <Estimate row={rest} loading={estimate.isPending} format={format} days={days} />
          </div>
        </li>
      </ul>

      <div aria-live="polite" className="space-y-2">
        {estimate.isError && <p className="text-sm text-muted-foreground">{t("estimateUnavailable")}</p>}
        {estimate.data?.days != null && (
          <p className="text-sm">
            {t.rich("total", { days: Math.ceil(estimate.data.days), b: (c) => <strong className="tabular-nums">{c}</strong> })}
          </p>
        )}
        {fragmented && (
          <div role="alert" className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning-foreground" aria-hidden />
            <div>
              <p className="font-medium">{t("fragmentedTitle")}</p>
              <p className="mt-0.5 text-pretty">
                {slow.length ? t("fragmentedSlow", { n: slow.length, max: TOO_LONG }) : t("fragmentedMany", { n: cells.length })}
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Estimate({
  row,
  loading,
  format,
  days,
}: {
  row: { visitors_per_day: number; days: number | null; too_small?: boolean } | undefined;
  loading: boolean;
  format: ReturnType<typeof useFormatter>;
  days: (n: number | null | undefined) => string;
}) {
  const t = useTranslations("xpWizard.audience");
  if (loading) return <div className="mt-3 h-5 w-48 animate-pulse rounded bg-muted motion-reduce:animate-none" />;
  if (!row) return null;
  const slow = row.too_small || (row.days ?? 0) > TOO_LONG;
  return (
    <p className={cn("mt-3 flex flex-wrap gap-x-4 gap-y-1 text-sm", slow && "text-warning-foreground")}>
      <span>
        <span className="font-medium tabular-nums">{format.number(Math.round(row.visitors_per_day))}</span> {t("perDay")}
      </span>
      <span>
        {t("needs")} <span className="font-medium tabular-nums">{days(row.days)}</span>
      </span>
      {slow && (
        <span className="inline-flex items-center gap-1">
          <TriangleAlert className="size-3.5" aria-hidden /> {t("merged")}
        </span>
      )}
    </p>
  );
}

function SegmentEditor({
  segment: s,
  index,
  locales,
  onChange,
  onRemove,
}: {
  segment: Segment;
  index: number;
  locales: string[];
  onChange: (p: Partial<Segment>) => void;
  onRemove: () => void;
}) {
  const t = useTranslations("xpWizard.audience");
  const uiLocale = useLocale();
  const localeName = useLocaleName();
  const [countries, setCountries] = useState((s.rules.country ?? []).join(", "));
  const region = new Intl.DisplayNames([uiLocale], { type: "region" });
  const set = (attr: SegmentAttr, values: string[]) => onChange({ rules: { ...s.rules, [attr]: values } });
  const toggle = (attr: SegmentAttr, v: string) => {
    const cur = s.rules[attr] ?? [];
    set(attr, cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v]);
  };
  const codes = (s.rules.country ?? []).filter((c) => /^[A-Z]{2}$/.test(c));
  const badCodes = (s.rules.country ?? []).filter((c) => !/^[A-Z]{2}$/.test(c));
  const base = `xp-seg-${s.id}`;

  const chips = (attr: SegmentAttr, values: readonly string[], label: (v: string) => string) => (
    <fieldset className="min-w-0">
      <legend className="mb-1.5 text-xs font-medium text-muted-foreground">{t(`attr.${attr}`)}</legend>
      <div className="flex flex-wrap gap-1.5">
        {values.map((v) => {
          const on = (s.rules[attr] ?? []).includes(v);
          return (
            <button
              key={v}
              type="button"
              aria-pressed={on}
              onClick={() => toggle(attr, v)}
              className={cn(
                "min-h-9 rounded-full border px-3 text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                on && "border-brand bg-brand/10 font-medium text-foreground",
              )}
            >
              {label(v)}
            </button>
          );
        })}
      </div>
    </fieldset>
  );

  return (
    <div className="space-y-4">
      <div className="flex items-end gap-2">
        <div className="grid min-w-0 flex-1 gap-1.5">
          <Label htmlFor={`${base}-name`}>{t("name", { n: index + 1 })}</Label>
          <Input id={`${base}-name`} value={s.name} onChange={(e) => onChange({ name: e.target.value })} placeholder={t("namePlaceholder")} className="h-9" />
        </div>
        <Button type="button" variant="ghost" size="icon-lg" onClick={onRemove} aria-label={t("remove", { name: s.name || t("name", { n: index + 1 }) })}>
          <Trash2 />
        </Button>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="grid gap-1.5">
          <Label htmlFor={`${base}-country`} className="text-xs text-muted-foreground">
            <Globe2 className="size-3.5" aria-hidden /> {t("attr.country")}
          </Label>
          <Input
            id={`${base}-country`}
            value={countries}
            onChange={(e) => {
              setCountries(e.target.value);
              set("country", e.target.value.toUpperCase().split(/[\s,;]+/).filter(Boolean));
            }}
            placeholder="DE, AT, CH"
            autoComplete="off"
            translate="no"
            className="h-9 font-mono"
            aria-invalid={badCodes.length > 0 || undefined}
            aria-describedby={`${base}-country-hint`}
          />
          <p id={`${base}-country-hint`} className={cn("text-xs", badCodes.length ? "text-destructive" : "text-muted-foreground")}>
            {badCodes.length
              ? t("countryBad", { codes: badCodes.join(", ") })
              : codes.length
                ? codes.map((c) => { try { return region.of(c) ?? c; } catch { return c; } }).join(", ")
                : t("countryHint")}
          </p>
        </div>
        {chips("locale", locales, (l) => localeName(l))}
        {chips("source", ATTRS.source, (v) => t(`source.${v as (typeof ATTRS.source)[number]}`))}
        {chips("device", ATTRS.device, (v) => t(`device.${v as (typeof ATTRS.device)[number]}`))}
        <div className="md:col-span-2">
          {chips("visitor", ATTRS.visitor, (v) => t(`visitor.${v as (typeof ATTRS.visitor)[number]}`))}
          <p className="mt-1.5 text-xs text-muted-foreground">{t("visitorHint")}</p>
        </div>
      </div>
    </div>
  );
}
