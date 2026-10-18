"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Eye, History, KeyRound, Loader2, Plus, RotateCcw, Save, Trash2, Undo2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { ErrorState, StatusBadge, useLocaleName } from "@/components/common";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { api, ApiError, qk, type Issue, type ReviewAnchor, type TextEntry, type TextResults } from "@/lib/api";
import { Ago } from "@/components/review/panel";
import { cn } from "@/lib/utils";

const DRY_RUN_DELAY_MS = 450;
const norm = (s: string) => s.replace(/\s+/g, " ").trim();

/** Top-level ICU arguments of a message: `{name}`, `{count, plural, …}` → ["name", "count"]. */
export function placeholders(src: string): string[] {
  const out = new Set<string>();
  let depth = 0;
  for (let i = 0; i < src.length; i++) {
    if (src[i] === "{") {
      if (depth === 0) {
        const m = /^\{\s*([\w.-]+)/.exec(src.slice(i));
        if (m) out.add(m[1]);
      }
      depth++;
    } else if (src[i] === "}") depth = Math.max(0, depth - 1);
  }
  return [...out];
}
const missingIn = (value: string, names: string[]) => names.filter((n) => !new RegExp(`\\{\\s*${n.replace(/[.-]/g, "\\$&")}\\s*[,}]`).test(value));

export type EditTarget = { key: string | null; text: string; anchor?: ReviewAnchor };

/**
 * Manual text editing (contract §6): every locale of one key side by side, validated as you type (dry run),
 * protected placeholders, live preview in the frame, history + undo, remove a translation, create a key from a hard-coded text.
 */
export function TextEditor({
  slug,
  target,
  frameLocale,
  onPreview,
  onShowLocale,
  onBack,
}: {
  slug: string;
  target: EditTarget;
  frameLocale: string;
  onPreview: (key: string, value: string | null) => void;
  onShowLocale: (locale: string) => void;
  onBack: () => void;
}) {
  const t = useTranslations("reviewPage.editor");
  const [key, setKey] = useState(target.key);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b px-2 py-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="rtl:-scale-x-100" /> {t("back")}
        </Button>
        <h2 className="text-sm font-medium">{t("title")}</h2>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {key ? (
          <KeyEditor key={key} slug={slug} k={key} frameLocale={frameLocale} onPreview={onPreview} onShowLocale={onShowLocale} />
        ) : (
          <FindOrCreate slug={slug} target={target} frameLocale={frameLocale} onKey={setKey} />
        )}
      </div>
    </div>
  );
}

function FindOrCreate({ slug, target, frameLocale, onKey }: { slug: string; target: EditTarget; frameLocale: string; onKey: (k: string) => void }) {
  const t = useTranslations("reviewPage.editor");
  const qc = useQueryClient();
  const text = norm(target.text);
  const found = useQuery({ queryKey: [...qk.keys(slug), "find", text], queryFn: () => api.keys(slug, { q: text, per_page: 5 }), enabled: !!text });
  const exact = found.data?.items.find((k) => norm(k.source) === text || norm(k.translations[frameLocale]?.value ?? "") === text);
  const [newKey, setNewKey] = useState("");
  const [description, setDescription] = useState("");
  const create = useMutation({
    mutationFn: () => api.createText(slug, { source: text, key: newKey.trim() || undefined, description: description.trim() || undefined, anchor: target.anchor }),
    onSuccess: (e: TextEntry) => {
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      toast.success(t("created", { key: e.key }));
      onKey(e.key);
    },
  });
  useEffect(() => {
    if (exact) onKey(exact.key);
  }, [exact?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  if (found.isPending && text) return <Skeleton className="m-4 h-32" />;
  const candidates = found.data?.items ?? [];
  return (
    <div className="space-y-5 p-4">
      <blockquote className="border-s-2 border-brand ps-3 text-sm">{text || t("noText")}</blockquote>
      {candidates.length > 0 && (
        <section className="space-y-2">
          <h3 className="text-sm font-medium">{t("candidates")}</h3>
          <ul className="space-y-1.5">
            {candidates.map((k) => (
              <li key={k.key}>
                <button
                  type="button"
                  onClick={() => onKey(k.key)}
                  className="w-full rounded-md border px-3 py-2 text-start text-sm transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <code dir="ltr" className="block truncate font-mono text-xs text-muted-foreground">{k.key}</code>
                  <span className="line-clamp-2">{k.source}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <form
        className="space-y-3 rounded-lg border border-dashed p-3"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <div>
          <h3 className="flex items-center gap-1.5 text-sm font-medium">
            <KeyRound aria-hidden className="size-4" /> {t("createTitle")}
          </h3>
          <p className="mt-1 text-xs text-pretty text-muted-foreground">{t("createBody")}</p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rv-new-key">{t("keyName")}</Label>
          <Input id="rv-new-key" dir="ltr" className="font-mono" value={newKey} onChange={(e) => setNewKey(e.target.value)} placeholder={t("keyPlaceholder")} aria-describedby="rv-new-key-hint" />
          <p id="rv-new-key-hint" className="text-xs text-muted-foreground">{t("keyHint")}</p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="rv-new-desc">{t("description")}</Label>
          <Input id="rv-new-desc" value={description} onChange={(e) => setDescription(e.target.value)} aria-describedby="rv-new-desc-hint" />
          <p id="rv-new-desc-hint" className="text-xs text-muted-foreground">{t("descriptionHint")}</p>
        </div>
        <Button type="submit" size="sm" disabled={create.isPending || !text}>
          {create.isPending ? <Loader2 className="animate-spin" /> : <Plus />} {t("createKey")}
        </Button>
      </form>
    </div>
  );
}

function KeyEditor({
  slug,
  k,
  frameLocale,
  onPreview,
  onShowLocale,
}: {
  slug: string;
  k: string;
  frameLocale: string;
  onPreview: (key: string, value: string | null) => void;
  onShowLocale: (locale: string) => void;
}) {
  const t = useTranslations("reviewPage.editor");
  const qc = useQueryClient();
  const q = useQuery({ queryKey: qk.text(slug, k), queryFn: () => api.text(slug, k) });
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [results, setResults] = useState<TextResults>({});
  const entry = q.data;
  const locales = useMemo(() => Object.keys(entry?.values ?? {}).filter((l) => l !== entry?.source_locale), [entry]);
  const names = useMemo(() => placeholders(entry?.source ?? ""), [entry?.source]);
  const value = (l: string) => drafts[l] ?? entry?.values[l]?.value ?? "";
  const dirty = locales.filter((l) => drafts[l] !== undefined && drafts[l] !== (entry?.values[l]?.value ?? ""));
  const changed = Object.fromEntries(dirty.map((l) => [l, drafts[l]]));
  const changedKey = JSON.stringify(changed);

  // Live validation: a dry run of exactly what Save would send.
  useEffect(() => {
    if (!dirty.length || dirty.some((l) => !drafts[l].trim())) return;
    const timer = setTimeout(() => {
      api
        .putText(slug, k, { values: changed, dry_run: true })
        .then((r) => setResults((cur) => ({ ...cur, ...r.results })))
        .catch(() => {});
    }, DRY_RUN_DELAY_MS);
    return () => clearTimeout(timer);
  }, [changedKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Live preview of the locale shown in the frame.
  const framed = drafts[frameLocale];
  useEffect(() => onPreview(k, framed !== undefined && framed !== entry?.values[frameLocale]?.value ? framed : null), [framed]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => onPreview(k, null), []); // eslint-disable-line react-hooks/exhaustive-deps

  const done = () => {
    qc.invalidateQueries({ queryKey: qk.text(slug, k) });
    qc.invalidateQueries({ queryKey: qk.keys(slug) });
    qc.invalidateQueries({ queryKey: qk.annotations(slug) });
  };
  const save = useMutation({
    meta: { silent: true },
    mutationFn: () => api.putText(slug, k, { values: changed }),
    onSuccess: (r) => {
      setResults(r.results);
      setDrafts({});
      done();
      toast.success(t("saved", { count: dirty.length }));
    },
    onError: (e) => {
      const d = e instanceof ApiError ? (e.details as { results?: TextResults } | undefined) : undefined;
      if (d?.results) setResults(d.results);
      toast.error(e instanceof ApiError && e.code === "invalid_translation" ? t("invalid") : e.message);
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!entry)
    return (
      <div className="space-y-3 p-4">
        <Skeleton className="h-10" />
        <Skeleton className="h-24" />
        <Skeleton className="h-24" />
      </div>
    );

  const blocking = dirty.some((l) => !drafts[l].trim() || missingIn(drafts[l], names).length > 0 || results[l]?.issues.some((i) => i.severity === "error"));
  return (
    <form
      className="space-y-4 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty.length && !blocking) save.mutate();
      }}
    >
      <div className="space-y-1">
        <code dir="ltr" className="block font-mono text-xs break-all text-muted-foreground">{entry.key}</code>
        <p className="text-sm">
          <span className="text-muted-foreground">{t("source", { locale: entry.source_locale })}</span> {entry.source}
        </p>
        {entry.description && <p className="text-xs text-muted-foreground">{entry.description}</p>}
        {names.length > 0 && <p className="text-xs text-muted-foreground">{t("protected")}</p>}
      </div>
      {locales.map((l) => (
        <LocaleField
          key={l}
          slug={slug}
          k={k}
          locale={l}
          entry={entry}
          value={value(l)}
          dirty={dirty.includes(l)}
          names={names}
          issues={results[l]?.issues ?? []}
          inFrame={l === frameLocale}
          onChange={(v) => setDrafts((d) => ({ ...d, [l]: v }))}
          onReset={() =>
            setDrafts((d) => {
              const { [l]: _, ...rest } = d; // eslint-disable-line @typescript-eslint/no-unused-vars
              return rest;
            })
          }
          onShow={() => onShowLocale(l)}
          onChanged={done}
        />
      ))}
      <div className="sticky bottom-0 -mx-4 flex items-center gap-2 border-t bg-background/95 px-4 py-3 backdrop-blur">
        <p className="flex-1 text-xs text-muted-foreground" role="status">
          {dirty.length ? t("unsaved", { count: dirty.length }) : t("upToDate")}
        </p>
        {dirty.length > 0 && (
          <Button type="button" variant="ghost" size="sm" onClick={() => setDrafts({})}>
            {t("discard")}
          </Button>
        )}
        <Button type="submit" size="sm" disabled={!dirty.length || blocking || save.isPending}>
          {save.isPending ? <Loader2 className="animate-spin" /> : <Save />} {save.isPending ? t("saving") : t("save", { count: dirty.length })}
        </Button>
      </div>
    </form>
  );
}

function LocaleField({
  slug,
  k,
  locale,
  entry,
  value,
  dirty,
  names,
  issues,
  inFrame,
  onChange,
  onReset,
  onShow,
  onChanged,
}: {
  slug: string;
  k: string;
  locale: string;
  entry: TextEntry;
  value: string;
  dirty: boolean;
  names: string[];
  issues: Issue[];
  inFrame: boolean;
  onChange: (v: string) => void;
  onReset: () => void;
  onShow: () => void;
  onChanged: () => void;
}) {
  const t = useTranslations("reviewPage.editor");
  const localeName = useLocaleName();
  const ref = useRef<HTMLTextAreaElement>(null);
  const [history, setHistory] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const tr = entry.values[locale];
  const id = `rv-text-${locale}`;
  const missing = dirty ? missingIn(value, names) : [];
  const empty = dirty && !value.trim();
  const all: Issue[] = [
    ...(empty ? [{ code: "empty", severity: "error" as const, message: t("empty") }] : []),
    ...missing.map((n) => ({ code: "placeholder", severity: "error" as const, message: t("missingPlaceholder", { name: `{${n}}` }) })),
    ...(dirty ? issues.filter((i) => !(i.code.includes("placeholder") && missing.length)) : []),
  ];
  const invalid = all.some((i) => i.severity === "error");

  const hist = useQuery({ queryKey: [...qk.text(slug, k), "history", locale], queryFn: () => api.textHistory(slug, k, locale), enabled: history });
  const revert = useMutation({
    mutationFn: (history_id?: string) => api.revertText(slug, k, { locale, history_id }),
    onSuccess: () => {
      onReset();
      onChanged();
      hist.refetch();
      toast.success(t("reverted", { locale }));
    },
  });
  const remove = useMutation({
    mutationFn: () => api.deleteText(slug, k, locale),
    onSuccess: () => {
      onReset();
      onChanged();
      toast.success(t("removed", { locale }), { action: { label: t("undo"), onClick: () => revert.mutate(undefined) } });
    },
  });

  const insert = (n: string) => {
    const el = ref.current;
    const token = `{${n}}`;
    const at = el?.selectionStart ?? value.length;
    onChange(value.slice(0, at) + token + value.slice(el?.selectionEnd ?? at));
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(at + token.length, at + token.length);
    });
  };

  return (
    <section className={cn("space-y-1.5 rounded-lg border p-3", dirty && "border-brand/60", invalid && "border-destructive")} aria-labelledby={`${id}-label`}>
      <div className="flex flex-wrap items-center gap-2">
        <Label id={`${id}-label`} htmlFor={id} className="font-medium">
          {localeName(locale)} <span className="font-mono text-xs text-muted-foreground">{locale}</span>
        </Label>
        {tr && <StatusBadge status={tr.status} />}
        {inFrame ? (
          <span className="ms-auto flex items-center gap-1 text-xs text-brand">
            <Eye aria-hidden className="size-3.5" /> {t("inFrame")}
          </span>
        ) : (
          <Button type="button" size="xs" variant="ghost" className="ms-auto" onClick={onShow}>
            <Eye /> {t("showInFrame")}
          </Button>
        )}
      </div>
      <Textarea
        ref={ref}
        id={id}
        rows={2}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        aria-invalid={invalid || undefined}
        aria-describedby={`${id}-msgs`}
        dir={locale.startsWith("ar") || locale.startsWith("he") ? "rtl" : undefined}
        lang={locale}
      />
      {names.length > 0 && (
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label={t("insertPlaceholder")}>
          {names.map((n) => (
            <Button
              key={n}
              type="button"
              size="xs"
              variant="outline"
              className={cn("font-mono", missing.includes(n) && "border-destructive text-destructive")}
              aria-label={t("insert", { name: `{${n}}` })}
              onClick={() => insert(n)}
            >
              {`{${n}}`}
            </Button>
          ))}
        </div>
      )}
      <div id={`${id}-msgs`}>
        {all.length > 0 ? (
          <ul className="space-y-0.5 text-xs" aria-live="polite">
            {all.map((i, n) => (
              <li key={n} className={i.severity === "error" ? "text-destructive" : "text-warning-foreground"}>
                {i.message}
              </li>
            ))}
          </ul>
        ) : (
          dirty && <p className="text-xs text-success-foreground">{t("valid")}</p>
        )}
      </div>
      <div className="flex flex-wrap gap-1">
        {dirty && (
          <Button type="button" size="xs" variant="ghost" onClick={onReset}>
            <RotateCcw /> {t("reset")}
          </Button>
        )}
        <Button type="button" size="xs" variant="ghost" aria-expanded={history} aria-controls={`${id}-history`} onClick={() => setHistory((h) => !h)}>
          <History /> {t("history")}
        </Button>
        <Button type="button" size="xs" variant="ghost" disabled={revert.isPending || !tr?.value} onClick={() => revert.mutate(undefined)}>
          <Undo2 className="rtl:-scale-x-100" /> {t("undoLast")}
        </Button>
        {tr && tr.status !== "missing" && (
          <Button type="button" size="xs" variant="ghost" className="text-destructive" onClick={() => setConfirm(true)}>
            <Trash2 /> {t("remove")}
          </Button>
        )}
      </div>
      {history && (
        <div id={`${id}-history`} className="rounded-md bg-muted/40 p-2">
          {hist.isPending ? (
            <Skeleton className="h-12" />
          ) : !hist.data?.length ? (
            <p className="text-xs text-muted-foreground">{t("noHistory")}</p>
          ) : (
            <ol className="space-y-2">
              {hist.data.map((h, i) => (
                <li key={h.id} className="flex items-start gap-2 text-xs">
                  <span className="min-w-0 flex-1">
                    <span className="block break-words text-foreground">{h.value}</span>
                    <span className="text-muted-foreground">
                      {h.author ?? t(`origin.${h.origin === "machine" ? "machine" : "human"}`)} · <Ago date={h.created_at} />
                    </span>
                  </span>
                  {i > 0 && (
                    <Button type="button" size="xs" variant="outline" disabled={revert.isPending} onClick={() => revert.mutate(h.id)}>
                      {t("restore")}
                    </Button>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      )}
      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("removeTitle", { locale: localeName(locale) })}</AlertDialogTitle>
            <AlertDialogDescription>{t("removeBody")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("keep")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => remove.mutate()}>
              {t("removeConfirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
