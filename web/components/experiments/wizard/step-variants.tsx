"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, CircleAlert, Loader2, PenLine, ShieldAlert, Sparkles, Trash2, TriangleAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useState, useSyncExternalStore } from "react";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { api, qk, type Project, type XpCellCheck, type XpGenerated, type XpVariant } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { INTENTS, MAX_VARIANTS, useDebounced, type Draft } from "./draft";
import { useKeyRows } from "./step-keys";

const RTL = new Set(["ar", "he", "fa", "ur"]);
type Values = XpVariant["values"]; // locale -> key -> text
const LETTERS = "ABCDEFG";

/**
 * Server draft + local edit buffer. Typing updates the buffer at once; it is flushed (PUT arm) after a pause,
 * then the server readiness check re-runs: that check is the single source of validation.
 */
export function useArms(slug: string, id: string | null, keys: string[], locales: string[]) {
  const qc = useQueryClient();
  const exp = useQuery({ queryKey: qk.xp(slug, id ?? ""), queryFn: () => api.xp(slug, id!), enabled: !!id });
  const [edits, setEdits] = useState<Record<number, Values>>({});
  const [saveError, setSaveError] = useState<unknown>(null);
  const flush = useDebounced(edits, 600);
  const pending = Object.keys(edits).length > 0;

  useEffect(() => {
    if (!id || Object.keys(flush).length === 0) return;
    let cancelled = false;
    Promise.all(Object.entries(flush).map(([arm, values]) => api.xpPutArm(slug, id, Number(arm), { values })))
      .then(async () => {
        if (cancelled) return;
        setSaveError(null);
        await qc.invalidateQueries({ queryKey: qk.xp(slug, id) });
        // Drop only what was saved; anything typed since stays buffered.
        setEdits((cur) => {
          const next: Record<number, Values> = {};
          for (const [arm, byLocale] of Object.entries(cur)) {
            for (const [loc, byKey] of Object.entries(byLocale)) {
              for (const [k, v] of Object.entries(byKey)) {
                if (flush[Number(arm)]?.[loc]?.[k] !== v) ((next[Number(arm)] ??= {})[loc] ??= {})[k] = v;
              }
            }
          }
          return next;
        });
      })
      .catch((e) => !cancelled && setSaveError(e));
    return () => {
      cancelled = true;
    };
  }, [flush, id, slug, qc]);

  const check = useQuery({
    queryKey: qk.xpCheck(slug, id ?? ""),
    queryFn: () => api.xpCheck(slug, id!),
    enabled: !!id && !pending && !!exp.data,
  });
  // A fresh check after each successful save (the arm query key is the parent of this one, but keep it explicit).
  const armsVersion = JSON.stringify(exp.data?.arms ?? []);
  useEffect(() => {
    if (id) qc.invalidateQueries({ queryKey: qk.xpCheck(slug, id) });
  }, [armsVersion, id, slug, qc]);

  const arms = (exp.data?.arms ?? []).filter((a) => a.arm >= 1);
  const value = (arm: number, locale: string, key: string) =>
    edits[arm]?.[locale]?.[key] ?? arms.find((a) => a.arm === arm)?.values[locale]?.[key] ?? "";
  const set = (arm: number, locale: string, key: string, v: string) =>
    setEdits((cur) => ({ ...cur, [arm]: { ...cur[arm], [locale]: { ...cur[arm]?.[locale], [key]: v } } }));
  const cell = (arm: number, key: string, locale: string): XpCellCheck | undefined =>
    check.data?.cells.find((c) => c.arm === arm && c.key === key && c.locale === locale);

  let empty = 0;
  for (const a of arms) for (const k of keys) for (const l of locales) if (!value(a.arm, l, k).trim()) empty++;
  const checked = pending || check.isFetching ? [] : (check.data?.cells.filter((c) => c.arm >= 1) ?? []);
  const errors = checked.filter((c) => !c.ok && c.value).length;

  return {
    exp,
    arms,
    value,
    set,
    cell,
    pending: pending || check.isFetching || (!!id && check.isPending),
    saveError,
    empty,
    errors,
    total: arms.length * keys.length * locales.length,
    checked: checked.length,
  };
}
export type ArmsState = ReturnType<typeof useArms>;

export function StepVariants({
  project,
  locales,
  draft: d,
  update,
  arms: s,
}: {
  project: Project;
  locales: string[];
  draft: Draft;
  update: (p: Partial<Draft>) => void;
  arms: ArmsState;
}) {
  const t = useTranslations("xpWizard.variants");
  const errMsg = useErrorMessage();
  const qc = useQueryClient();
  const slug = project.slug;
  const id = d.id!;
  const [policy, setPolicy] = useState<XpGenerated | null>(null);
  const [custom, setCustom] = useState("");
  const free = MAX_VARIANTS - s.arms.length;
  const refresh = () => qc.invalidateQueries({ queryKey: qk.xp(slug, id) });

  const generate = useMutation({
    mutationFn: () => api.xpGenerate(slug, id, { intents: d.intents.slice(0, Math.max(1, free)), locales }),
    onSuccess: (r) => {
      setPolicy(r.rejected.length || r.policy?.length ? r : null);
      refresh();
    },
  });
  const addManual = useMutation({
    mutationFn: () => {
      const next = Math.max(0, ...s.arms.map((a) => a.arm)) + 1;
      return api.xpPutArm(slug, id, next, { intent: custom.trim() || t("handWritten"), values: {} });
    },
    onSuccess: () => {
      setCustom("");
      refresh();
    },
  });
  const remove = useMutation({ mutationFn: (arm: number) => api.xpDeleteArm(slug, id, arm), onSuccess: refresh });
  const rename = useMutation({ mutationFn: ({ arm, intent }: { arm: number; intent: string }) => api.xpPutArm(slug, id, arm, { intent }), onSuccess: refresh });

  const intentLabel = (i: string) => ((INTENTS as readonly string[]).includes(i) ? t(`intent.${i as (typeof INTENTS)[number]}.label`) : i);
  const toggleIntent = (i: string) => update({ intents: d.intents.includes(i) ? d.intents.filter((x) => x !== i) : [...d.intents, i] });
  const busy = generate.isPending || addManual.isPending;

  return (
    <div className="space-y-6">
      {/* 1. Intentions */}
      <fieldset className="space-y-3">
        <legend className="text-sm font-medium">{t("intents")}</legend>
        <p className="-mt-1 text-xs text-muted-foreground">{t("intentsHint")}</p>
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {INTENTS.map((i) => {
            const on = d.intents.includes(i);
            return (
              <button
                key={i}
                type="button"
                aria-pressed={on}
                onClick={() => toggleIntent(i)}
                className={cn(
                  "flex min-h-11 items-start gap-2.5 rounded-lg border p-3 text-start text-sm transition-colors duration-150 outline-none hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                  on && "border-brand bg-brand/5",
                )}
              >
                <span aria-hidden className={cn("mt-0.5 grid size-4 shrink-0 place-items-center rounded-[4px] border", on && "border-brand bg-brand text-brand-foreground")}>
                  {on && <Check className="size-3" />}
                </span>
                <span>
                  <span className="block font-medium">{t(`intent.${i}.label`)}</span>
                  <span className="block text-xs text-muted-foreground">{t(`intent.${i}.hint`)}</span>
                </span>
              </button>
            );
          })}
        </div>
        {d.intents.includes("social_proof") && (
          <div className="grid gap-1.5">
            <Label htmlFor="xp-facts">{t("facts")}</Label>
            <Textarea id="xp-facts" value={d.facts} onChange={(e) => update({ facts: e.target.value })} rows={2} placeholder={t("factsPlaceholder")} aria-describedby="xp-facts-hint" />
            <p id="xp-facts-hint" className="text-xs text-muted-foreground">
              {t("factsHint")}
            </p>
          </div>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" size="lg" onClick={() => generate.mutate()} disabled={busy || free <= 0 || d.intents.length === 0} aria-busy={generate.isPending}>
            {generate.isPending ? <Loader2 className="animate-spin" /> : <Sparkles />}
            {generate.isPending ? t("generating") : t("generate", { n: Math.min(Math.max(free, 0), d.intents.length) })}
          </Button>
          <span className="text-xs text-muted-foreground">{t("or")}</span>
          <div className="flex min-w-0 basis-full items-center gap-2 sm:max-w-md sm:flex-1 sm:basis-auto">
            <Label htmlFor="xp-manual" className="sr-only">
              {t("manualLabel")}
            </Label>
            <Input
              id="xp-manual"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  if (free > 0) addManual.mutate();
                }
              }}
              placeholder={t("manualPlaceholder")}
              className="h-9 min-w-0 flex-1"
            />
            <Button type="button" variant="outline" size="lg" onClick={() => addManual.mutate()} disabled={busy || free <= 0}>
              <PenLine /> {t("manual")}
            </Button>
          </div>
        </div>
        {free <= 0 && <p className="text-xs text-muted-foreground">{t("maxArms", { n: MAX_VARIANTS })}</p>}
        <p role="status" className="sr-only">
          {generate.isPending ? t("generating") : generate.isSuccess ? t("generated", { n: generate.data.arms.length }) : ""}
        </p>
        {(generate.isError || addManual.isError || remove.isError) && (
          <p role="alert" className="flex items-start gap-2 text-sm text-destructive">
            <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden /> {errMsg(generate.error ?? addManual.error ?? remove.error)}
          </p>
        )}
      </fieldset>

      {policy && <PolicyAlert result={policy} intentLabel={intentLabel} />}

      {/* 2. Grid */}
      {s.exp.isPending ? (
        <div className="h-40 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />
      ) : s.arms.length === 0 ? (
        <div className="rounded-lg border border-dashed p-6 text-center text-sm text-pretty text-muted-foreground">{t("empty")}</div>
      ) : (
        <>
          <ValidationSummary s={s} />
          <Grid project={project} locales={locales} keys={d.keys} s={s} intentLabel={intentLabel} onRemove={(a) => remove.mutate(a)} onRename={(arm, intent) => rename.mutate({ arm, intent })} />
        </>
      )}
    </div>
  );
}

function PolicyAlert({ result, intentLabel }: { result: XpGenerated; intentLabel: (i: string) => string }) {
  const t = useTranslations("xpWizard.variants");
  const blocking = result.policy?.some((p) => p.level === "block");
  return (
    <div role="alert" className={cn("rounded-lg border p-3 text-sm", blocking ? "border-destructive/40 bg-destructive/5" : "border-warning/40 bg-warning/10")}>
      <p className="flex items-center gap-2 font-medium">
        <ShieldAlert className={cn("size-4", blocking ? "text-destructive" : "text-warning-foreground")} aria-hidden /> {t("policyTitle")}
      </p>
      <ul className="mt-1.5 list-disc space-y-1 ps-6 text-pretty">
        {result.policy?.map((p, i) => (
          <li key={`p${i}`}>
            {p.arm ? <span className="font-medium">{t("variantName", { letter: LETTERS[p.arm - 1] ?? p.arm })}: </span> : null}
            {p.message}
          </li>
        ))}
        {result.rejected.map((r, i) => (
          <li key={`r${i}`}>
            {r.intent && <span className="font-medium">{intentLabel(r.intent)}: </span>}
            {r.reason ?? r.code}
            {r.text && <span className="text-muted-foreground"> — “{r.text}”</span>}
          </li>
        ))}
      </ul>
      <p className="mt-1.5 text-xs text-muted-foreground">{t("policyHint")}</p>
    </div>
  );
}

/** Live (polite) one-line summary of the validator, so screen readers hear the result without leaving the cell. */
function ValidationSummary({ s }: { s: ArmsState }) {
  const t = useTranslations("xpWizard.variants");
  const state = s.saveError ? "saveError" : s.pending ? "checking" : s.empty > 0 ? "empty" : s.errors > 0 ? "errors" : "ok";
  const Icon = state === "checking" ? Loader2 : state === "ok" ? Check : TriangleAlert;
  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "flex items-center gap-2 rounded-md px-3 py-2 text-sm",
        state === "ok" && "bg-success/10 text-success-foreground",
        (state === "errors" || state === "saveError") && "bg-destructive/5 text-destructive",
        (state === "empty" || state === "checking") && "bg-muted text-muted-foreground",
      )}
    >
      <Icon className={cn("size-4 shrink-0", state === "checking" && "animate-spin")} aria-hidden />
      {t(`summary.${state}`, { n: state === "empty" ? s.empty : s.errors, total: s.total })}
    </div>
  );
}

function Grid({
  project,
  locales,
  keys,
  s,
  intentLabel,
  onRemove,
  onRename,
}: {
  project: Project;
  locales: string[];
  keys: string[];
  s: ArmsState;
  intentLabel: (i: string) => string;
  onRemove: (arm: number) => void;
  onRename: (arm: number, intent: string) => void;
}) {
  const t = useTranslations("xpWizard.variants");
  const name = useLocaleName();
  const rows = useKeyRows(project.slug, keys);
  const fits = s.exp.data?.fits ?? {};
  const wide = useSyncExternalStore(subscribeWide, () => matchMedia(WIDE).matches, () => true);
  const official = (k: string, ki: number, l: string) => (l === project.source_locale ? rows[ki]?.data?.source : rows[ki]?.data?.translations[l]?.value);
  const budget = (k: string) => fits[k]?.max_width_px && <span className="ms-2 font-normal text-muted-foreground">{t("budget", { px: fits[k].max_width_px!, lines: fits[k].max_lines ?? 1 })}</span>;
  const cell = (a: XpVariant, k: string, ki: number, l: string) => (
    <Cell
      id={`xp-c-${a.arm}-${ki}-${l}`}
      label={t("cellLabel", { letter: LETTERS[a.arm - 1] ?? a.arm, key: k, locale: name(l) })}
      locale={l}
      value={s.value(a.arm, l, k)}
      onChange={(v) => s.set(a.arm, l, k, v)}
      check={s.pending ? undefined : s.cell(a.arm, k, l)}
      pending={s.pending}
    />
  );

  // Phones: one card per variant, languages stacked (a 3-column table can't breathe at 375 px).
  if (!wide)
    return (
      <div className="space-y-4" role="region" aria-label={t("gridLabel")}>
        {s.arms.map((a) => (
          <section key={a.arm} className="rounded-lg border">
            <div className="border-b px-3 py-2 text-sm font-medium">
              <ArmHeader arm={a} intentLabel={intentLabel} onRemove={onRemove} onRename={onRename} />
            </div>
            {keys.map((k, ki) => (
              <div key={k} className="space-y-3 border-b p-3 last:border-b-0">
                <p className="text-xs">
                  <span className="font-mono font-medium" translate="no">
                    {k}
                  </span>
                  {budget(k)}
                </p>
                {locales.map((l) => (
                  <div key={l}>
                    <p className="mb-1 flex gap-2 text-xs text-muted-foreground">
                      <span className="font-mono font-medium text-foreground">{l}</span>
                      <span className="min-w-0 truncate" lang={l}>
                        {t("officialShort")} {official(k, ki, l) || t("noOfficial")}
                      </span>
                    </p>
                    {cell(a, k, ki, l)}
                  </div>
                ))}
              </div>
            ))}
          </section>
        ))}
      </div>
    );

  return (
    <div className="overflow-x-auto rounded-lg border" role="region" aria-label={t("gridLabel")} tabIndex={0}>
      <table className="w-full min-w-[40rem] border-separate border-spacing-0 text-sm">
        <thead>
          <tr className="text-xs text-muted-foreground">
            <th scope="col" className="sticky start-0 z-10 w-16 border-b bg-card px-3 py-2 text-start font-medium">
              {t("locale")}
            </th>
            <th scope="col" className="min-w-48 border-b bg-muted/40 px-3 py-2 text-start font-medium">
              {t("official")}
            </th>
            {s.arms.map((a) => (
              <th key={a.arm} scope="col" className="min-w-60 border-b px-3 py-2 text-start align-bottom font-medium">
                <ArmHeader arm={a} intentLabel={intentLabel} onRemove={onRemove} onRename={onRename} />
              </th>
            ))}
          </tr>
        </thead>
        {keys.map((k, ki) => {
          return (
            <tbody key={k}>
              <tr>
                <th scope="colgroup" colSpan={2 + s.arms.length} className="border-b bg-muted/30 px-3 py-1.5 text-start text-xs font-normal">
                  <span className="font-mono font-medium text-foreground" translate="no">
                    {k}
                  </span>
                  {budget(k)}
                </th>
              </tr>
              {locales.map((l) => {
                return (
                  <tr key={l} className="align-top">
                    <th scope="row" className="sticky start-0 z-10 border-b bg-card px-3 py-2.5 text-start font-mono text-xs font-medium" title={name(l)}>
                      {l}
                    </th>
                    <td lang={l} dir={RTL.has(l) ? "rtl" : undefined} className="border-b bg-muted/40 px-3 py-2.5 text-pretty text-muted-foreground">
                      {official(k, ki, l) || <span className="italic">{t("noOfficial")}</span>}
                    </td>
                    {s.arms.map((a) => (
                      <td key={a.arm} className="border-b px-2 py-1.5">
                        {cell(a, k, ki, l)}
                      </td>
                    ))}
                  </tr>
                );
              })}
            </tbody>
          );
        })}
      </table>
    </div>
  );
}

const WIDE = "(min-width: 768px)";
const subscribeWide = (cb: () => void) => {
  const m = matchMedia(WIDE);
  m.addEventListener("change", cb);
  return () => m.removeEventListener("change", cb);
};

function ArmHeader({
  arm: a,
  intentLabel,
  onRemove,
  onRename,
}: {
  arm: XpVariant;
  intentLabel: (i: string) => string;
  onRemove: (arm: number) => void;
  onRename: (arm: number, intent: string) => void;
}) {
  const t = useTranslations("xpWizard.variants");
  const letter = LETTERS[a.arm - 1] ?? a.arm;
  return (
    <div className="flex items-center gap-1.5">
      <span className="shrink-0 text-foreground">{t("variantName", { letter })}</span>
      <Input
        aria-label={t("intentOf", { letter })}
        defaultValue={intentLabel(a.intent)}
        onBlur={(e) => e.target.value.trim() && e.target.value !== intentLabel(a.intent) && onRename(a.arm, e.target.value.trim())}
        className="h-8 min-w-0 flex-1 border-transparent bg-transparent px-1.5 text-xs text-muted-foreground shadow-none hover:border-input focus-visible:text-foreground"
      />
      <Button type="button" variant="ghost" size="icon" onClick={() => onRemove(a.arm)} aria-label={t("remove", { letter })}>
        <Trash2 />
      </Button>
    </div>
  );
}

function Cell({
  id,
  label,
  locale,
  value,
  onChange,
  check,
  pending,
}: {
  id: string;
  label: string;
  locale: string;
  value: string;
  onChange: (v: string) => void;
  check: XpCellCheck | undefined;
  pending: boolean;
}) {
  const t = useTranslations("xpWizard.variants");
  const empty = !value.trim();
  const bad = !pending && !!check && !check.ok && !empty;
  const good = !pending && !!check && check.ok && !empty;
  const issues = check?.issues ?? [];
  return (
    <div>
      <Textarea
        id={id}
        aria-label={label}
        lang={locale}
        dir={RTL.has(locale) ? "rtl" : undefined}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        rows={2}
        aria-invalid={bad || undefined}
        aria-describedby={issues.length && !empty ? `${id}-m` : undefined}
        className={cn(
          "min-h-14 resize-y text-sm transition-colors duration-150",
          good && "border-success/70 dark:border-success/60",
          bad && "border-destructive",
        )}
      />
      <div id={`${id}-m`} className="mt-1 min-h-4 text-xs">
        {empty ? (
          <span className="text-muted-foreground">{t("cellEmpty")}</span>
        ) : pending || !check ? (
          <span className="text-muted-foreground">{t("cellChecking")}</span>
        ) : issues.length === 0 ? (
          <span className="inline-flex items-center gap-1 text-success-foreground">
            <Check className="size-3" aria-hidden /> {t("cellOk")}
          </span>
        ) : (
          <ul className="space-y-0.5">
            {issues.map((i, n) => (
              <li key={n} className={cn("flex items-start gap-1", i.severity === "error" ? "text-destructive" : "text-warning-foreground")}>
                {i.severity === "error" ? <CircleAlert className="mt-px size-3 shrink-0" aria-hidden /> : <TriangleAlert className="mt-px size-3 shrink-0" aria-hidden />}
                <span>{i.message}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

