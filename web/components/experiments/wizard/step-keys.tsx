"use client";

import { useQueries, useQuery } from "@tanstack/react-query";
import { Grid3x3, Layers, Search, X } from "lucide-react";
import { useTranslations } from "next-intl";
import { useId, useState } from "react";
import { useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api, ApiError, qk, type KeyRow, type Project } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useDebounced } from "./draft";

const prefixOf = (k: string) => k.split(".").slice(0, -1).join(".");
const RTL = new Set(["ar", "he", "fa", "ur"]);

/** Exact row for a key (the list endpoint searches, so filter on the exact match). */
export function useKeyRows(slug: string, keys: string[]) {
  return useQueries({
    queries: keys.map((key) => ({
      queryKey: [...qk.keys(slug), "exact", key],
      queryFn: () => api.keys(slug, { q: key, per_page: 50 }).then((p) => p.items.find((r) => r.key === key) ?? null),
      staleTime: 60_000,
    })),
  });
}

export function StepKeys({ project, keys, onChange }: { project: Project; keys: string[]; onChange: (keys: string[]) => void }) {
  const t = useTranslations("xpWizard.keys");
  const slug = project.slug;
  const [q, setQ] = useState("");
  const dq = useDebounced(q.trim(), 250);
  const listId = useId();
  const search = useQuery({
    queryKey: [...qk.keys(slug), "xp-search", dq],
    queryFn: () => api.keys(slug, { q: dq || undefined, per_page: 40 }),
    placeholderData: (prev) => prev,
  });
  const items = search.data?.items ?? [];

  // Groups = prefixes shared by ≥ 2 keys in the results; heroes first (title + subtitle + CTA in one click).
  const groups = Object.entries(
    items.reduce<Record<string, string[]>>((m, r) => {
      const p = prefixOf(r.key);
      if (p) (m[p] ??= []).push(r.key);
      return m;
    }, {}),
  )
    .filter(([, ks]) => ks.length >= 2 && ks.length <= 6)
    .sort(([a], [b]) => Number(b.includes("hero")) - Number(a.includes("hero")) || a.length - b.length)
    .slice(0, 4);

  const toggle = (k: string, on: boolean) => onChange(on ? [...new Set([...keys, k])] : keys.filter((x) => x !== k));

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="min-w-0 space-y-3">
        <div className="grid gap-1.5">
          <Label htmlFor="xp-key-search">{t("search")}</Label>
          <div className="relative">
            <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
            <Input
              id="xp-key-search"
              type="search"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && e.preventDefault()}
              placeholder="home.hero"
              autoComplete="off"
              translate="no"
              className="h-10 ps-8 font-mono"
              aria-describedby={`${listId}-hint`}
              aria-controls={listId}
            />
          </div>
          <p id={`${listId}-hint`} className="text-xs text-muted-foreground">
            {t("searchHint")}
          </p>
        </div>

        {groups.length > 0 && (
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">{t("groups")}</p>
            <div className="flex flex-wrap gap-2">
              {groups.map(([prefix, ks]) => {
                const all = ks.every((k) => keys.includes(k));
                return (
                  <Button
                    key={prefix}
                    type="button"
                    variant={all ? "secondary" : "outline"}
                    size="lg"
                    aria-pressed={all}
                    onClick={() => onChange(all ? keys.filter((k) => !ks.includes(k)) : [...new Set([...keys, ...ks])])}
                    className="max-w-full"
                  >
                    <Layers aria-hidden />
                    <span className="truncate font-mono" translate="no">
                      {prefix}.*
                    </span>
                    <span className={cn("tabular-nums", !all && "text-muted-foreground")}>{t("groupCount", { n: ks.length })}</span>
                  </Button>
                );
              })}
            </div>
          </div>
        )}

        <div
          id={listId}
          role="group"
          aria-label={t("results")}
          aria-busy={search.isFetching}
          className="max-h-[22rem] overflow-y-auto rounded-lg border"
        >
          {search.isPending ? (
            <div className="space-y-2 p-3">
              {Array.from({ length: 5 }, (_, i) => (
                <Skeleton key={i} className="h-9" />
              ))}
            </div>
          ) : items.length === 0 ? (
            <p className="p-6 text-center text-sm text-muted-foreground">{dq ? t("noMatch", { q: dq }) : t("noKeys")}</p>
          ) : (
            <ul className="divide-y">
              {items.map((r) => {
                const id = `${listId}-${r.key}`;
                const on = keys.includes(r.key);
                return (
                  <li key={r.key}>
                    <label htmlFor={id} className={cn("flex min-h-11 cursor-pointer items-center gap-3 px-3 py-2 hover:bg-muted/60", on && "bg-brand/5")}>
                      <Checkbox id={id} checked={on} onCheckedChange={(v) => toggle(r.key, v === true)} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-mono text-[13px]" translate="no" title={r.key}>
                          {r.key}
                        </span>
                        <span className="block truncate text-xs text-muted-foreground" lang={project.source_locale}>
                          {r.source}
                        </span>
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        {search.data && search.data.total > items.length && (
          <p className="text-xs text-muted-foreground">{t("more", { shown: items.length, total: search.data.total })}</p>
        )}
      </div>

      <InContext project={project} keys={keys} onRemove={(k) => toggle(k, false)} />
    </div>
  );
}

/** Selected keys as a mini locale matrix + any capture the last visual QA run took of them. */
function InContext({ project, keys, onRemove }: { project: Project; keys: string[]; onRemove: (k: string) => void }) {
  const t = useTranslations("xpWizard.keys");
  const name = useLocaleName();
  const slug = project.slug;
  const rows = useKeyRows(slug, keys);
  const report = useQuery({
    queryKey: qk.visualReport(slug, "latest"),
    queryFn: () => api.visualReport(slug, "latest").catch((e) => (e instanceof ApiError && e.status === 404 ? null : Promise.reject(e))),
    staleTime: 5 * 60_000,
  });
  const locales = [project.source_locale, ...project.target_locales];
  const crops = (key: string) =>
    (report.data?.report.issues ?? []).filter((i) => i.key === key && i.crop).slice(0, 2);

  return (
    <section aria-labelledby="xp-selected" className="min-w-0 space-y-3">
      <div className="flex items-baseline justify-between gap-2">
        <h3 id="xp-selected" className="text-sm font-medium">
          {t("selected", { n: keys.length })}
        </h3>
        <Link href={`/app/${slug}/preview`} className="inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
          <Grid3x3 className="size-3.5" aria-hidden /> {t("openMatrix")}
        </Link>
      </div>
      {keys.length === 0 ? (
        <p className="rounded-lg border border-dashed p-6 text-center text-sm text-pretty text-muted-foreground">{t("emptySelected")}</p>
      ) : (
        <ul className="space-y-3">
          {keys.map((k, i) => {
            const row: KeyRow | null | undefined = rows[i]?.data;
            return (
              <li key={k} className="rounded-lg border motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-150">
                <div className="flex items-center gap-2 border-b px-3 py-1.5">
                  <span className="min-w-0 flex-1 truncate font-mono text-[13px] font-medium" translate="no" title={k}>
                    {k}
                  </span>
                  <Button type="button" variant="ghost" size="icon" onClick={() => onRemove(k)} aria-label={t("remove", { key: k })}>
                    <X />
                  </Button>
                </div>
                {row === undefined ? (
                  <Skeleton className="m-3 h-12" />
                ) : (
                  <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 px-3 py-2 text-sm">
                    {locales.map((l) => {
                      const v = l === project.source_locale ? row?.source : row?.translations[l]?.value;
                      return (
                        <div key={l} className="contents">
                          <dt className="font-mono text-xs leading-5 text-muted-foreground" title={name(l)}>
                            {l}
                          </dt>
                          <dd lang={l} dir={RTL.has(l) ? "rtl" : undefined} className={cn("truncate leading-5", !v && "text-muted-foreground italic")} title={v ?? undefined}>
                            {v || t("missing")}
                          </dd>
                        </div>
                      );
                    })}
                  </dl>
                )}
                {report.data &&
                  crops(k).map((c) => (
                    // eslint-disable-next-line @next/next/no-img-element -- authenticated API file, not optimizable
                    <img
                      key={`${c.locale}-${c.route}-${c.viewport}`}
                      src={api.visualFileUrl(slug, report.data!.id, c.crop!)}
                      alt={t("cropAlt", { key: k, locale: name(c.locale), route: c.route })}
                      loading="lazy"
                      className="mx-3 mb-3 max-h-24 max-w-[calc(100%-1.5rem)] rounded border object-contain object-left"
                    />
                  ))}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
