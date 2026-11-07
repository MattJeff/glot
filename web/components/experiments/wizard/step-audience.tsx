"use client";

import { Globe2, Plus, Trash2, TriangleAlert, Users } from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { useState } from "react";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { DurationEstimate } from "@/components/experiments/duration";
import type { Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { ATTRS, MAX_SEGMENTS, segmentsToCells, uid, useDebounced, type Draft, type Segment, type SegmentAttr } from "./draft";

const PRESETS = [
  { name: "mobile", rules: { device: ["mobile"] } },
  { name: "search", rules: { source: ["search"] } },
  { name: "new", rules: { visitor: ["new"] } },
] as const satisfies { name: string; rules: Segment["rules"] }[];

export function StepAudience({
  project,
  locales,
  allLocales,
  draft: d,
  update,
  armCount,
}: {
  project: Project;
  locales: string[];
  allLocales: string[];
  draft: Draft;
  update: (p: Partial<Draft>) => void;
  armCount: number;
}) {
  const t = useTranslations("xpWizard.audience");
  const tl = useTranslations("xpLifecycle.wizard");
  const localeName = useLocaleName();
  const setSegments = (segments: Segment[]) => update({ segments });
  // An edited segment is sent as {name, rule} again (no longer by server id).
  const patch = (id: string, p: Partial<Segment>) => setSegments(d.segments.map((s) => (s.id === id ? { ...s, ...p, ref: undefined } : s)));
  const add = (preset?: (typeof PRESETS)[number]) =>
    setSegments([...d.segments, { id: uid(), name: preset ? t(`preset.${preset.name}`) : "", rules: preset ? { ...preset.rules } : {} }]);

  const cells = useDebounced(segmentsToCells(d.segments, t("unnamed")), 500);
  const fragmented = cells.length >= 3;

  const toggleLocale = (l: string) =>
    update({ locales: locales.includes(l) ? locales.filter((x) => x !== l) : allLocales.filter((x) => x === l || locales.includes(x)) });

  return (
    <div className="space-y-5">
      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">{tl("locales")}</legend>
        <p className="text-xs text-pretty text-muted-foreground">{tl("localesHint")}</p>
        <div className="flex flex-wrap gap-1.5">
          {allLocales.map((l) => {
            const on = locales.includes(l);
            return (
              <button
                key={l}
                type="button"
                aria-pressed={on}
                disabled={on && locales.length === 1}
                onClick={() => toggleLocale(l)}
                className={cn(
                  "min-h-9 rounded-full border px-3 text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed",
                  on && "border-brand bg-brand/10 font-medium text-foreground",
                )}
              >
                {localeName(l)} <span className="font-mono text-xs text-muted-foreground">{l}</span>
              </button>
            );
          })}
        </div>
      </fieldset>

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
          </li>
        ))}
        <li className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-dashed p-3 sm:p-4">
          <Users className="size-4 text-muted-foreground" aria-hidden />
          <span className="font-medium">{d.segments.length ? t("everyoneElse") : t("everyone")}</span>
          <span className="text-sm text-muted-foreground">{d.segments.length ? t("everyoneElseHint") : t("everyoneHint")}</span>
        </li>
      </ul>

      <DurationEstimate slug={project.slug} segments={cells} arms={armCount + 1} goal={d.goal} minN={d.minN} trafficPct={d.trafficPct} />

      <div aria-live="polite" className="space-y-2">
        {fragmented && (
          <div role="alert" className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning-foreground" aria-hidden />
            <div>
              <p className="font-medium">{t("fragmentedTitle")}</p>
              <p className="mt-0.5 text-pretty">
                {t("fragmentedMany", { n: cells.length })}
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
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
