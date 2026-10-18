"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, Loader2, SearchX } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { StatusBadge } from "@/components/common";
import { ReviewBlock } from "@/components/review";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { Link } from "@/i18n/navigation";
import { api, type KeyRow, type Project, qk } from "@/lib/api";
import { cn } from "@/lib/utils";

export type KeyTarget = { locale: string; key?: string; text?: string; issue?: string };

const norm = (s: string) => s.replace(/\s+/g, " ").trim().toLowerCase();

/** Finds the key behind an on-screen issue: by key when the app tags elements (data-glot-key), else by rendered text. */
function useResolvedKeys(project: Project, target: KeyTarget | null) {
  const slug = project.slug;
  return useQuery({
    queryKey: [...qk.keys(slug), "resolve", target],
    enabled: !!target,
    queryFn: async (): Promise<KeyRow[]> => {
      if (!target) return [];
      if (target.key) {
        const page = await api.keys(slug, { q: target.key, per_page: 20 });
        return page.items.filter((r) => r.key === target.key);
      }
      // ponytail: scans the first 500 keys of the locale; a server-side value search would scale further.
      const page = await api.keys(slug, { locale: target.locale, per_page: 500 });
      const text = norm(target.text ?? "");
      if (!text) return [];
      const value = (r: KeyRow) => norm(target.locale === project.source_locale ? r.source : (r.translations[target.locale]?.value ?? ""));
      const exact = page.items.filter((r) => value(r) === text);
      if (exact.length) return exact;
      return page.items.filter((r) => value(r).length > 2 && text.includes(value(r))).slice(0, 8);
    },
  });
}

export function KeySheet({
  project,
  target,
  onClose,
}: {
  project: Project;
  target: KeyTarget | null;
  onClose: () => void;
}) {
  const t = useTranslations("keySheet");
  const resolved = useResolvedKeys(project, target);
  const [picked, setPicked] = useState<string | null>(null);
  const rows = resolved.data ?? [];
  const row = rows.length === 1 ? rows[0] : rows.find((r) => r.key === picked);

  return (
    <Sheet
      open={!!target}
      onOpenChange={(o) => {
        if (!o) {
          setPicked(null);
          onClose();
        }
      }}
    >
      <SheetContent className="w-full gap-0 overflow-y-auto sm:max-w-md">
        <SheetHeader className="border-b">
          <SheetTitle>{t("title")}</SheetTitle>
          <SheetDescription className="font-mono text-xs">{target?.locale}</SheetDescription>
          {target?.issue && <p className="rounded-md bg-destructive/10 px-2 py-1.5 text-xs text-destructive">{target.issue}</p>}
        </SheetHeader>
        {resolved.isPending ? (
          <div className="space-y-3 p-4">
            <Skeleton className="h-5 w-48" />
            <Skeleton className="h-24" />
          </div>
        ) : row && target ? (
          <KeyForm key={`${row.key}:${target.locale}`} project={project} row={row} locale={target.locale} />
        ) : rows.length > 1 ? (
          <div className="p-4">
            <p className="mb-2 text-sm text-muted-foreground">{t("pick")}</p>
            <ul className="divide-y rounded-lg border">
              {rows.map((r) => (
                <li key={r.key}>
                  <button type="button" onClick={() => setPicked(r.key)} className="block w-full px-3 py-2 text-start hover:bg-accent/50">
                    <span className="block truncate font-mono text-xs">{r.key}</span>
                    <span className="block truncate text-xs text-muted-foreground">{r.source}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2 p-8 text-center">
            <SearchX className="size-5 text-muted-foreground" />
            <p className="text-sm font-medium">{t("notFound")}</p>
            <p className="text-xs text-muted-foreground">{t("notFoundHint")}</p>
            {target?.text && (
              <p dir="auto" className="mt-2 rounded-md border bg-muted/40 px-2 py-1 text-xs">
                {target.text}
              </p>
            )}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

function KeyForm({ project, row, locale }: { project: Project; row: KeyRow; locale: string }) {
  const t = useTranslations("keySheet");
  const tc = useTranslations("common");
  const qc = useQueryClient();
  const isSource = locale === project.source_locale;
  const tr = row.translations[locale];
  const [value, setValue] = useState(isSource ? row.source : (tr?.value ?? ""));
  const save = useMutation({
    mutationFn: () => api.putTranslation(project.slug, row.key, locale, value),
    onSuccess: (next) => {
      qc.invalidateQueries({ queryKey: qk.keys(project.slug) });
      qc.invalidateQueries({ queryKey: qk.project(project.slug) });
      if (next.issues.length) toast.warning(t("savedWithIssues", { count: next.issues.length }));
      else toast.success(t("saved"));
    },
  });

  return (
    <div className="space-y-5 p-4 text-sm">
      <div>
        <p className="font-mono text-xs break-all">{row.key}</p>
        {row.description && <p className="mt-1 text-xs text-muted-foreground">{row.description}</p>}
      </div>
      <div>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground">
          {t("source")} <span className="font-mono">{project.source_locale}</span>
        </h3>
        <p dir="auto" lang={project.source_locale} className="whitespace-pre-wrap">
          {row.source}
        </p>
      </div>
      {!isSource && (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <h3 className="text-xs font-medium text-muted-foreground">
              {t("translation")} <span className="font-mono">{locale}</span>
            </h3>
            {tr && <StatusBadge status={tr.status} />}
          </div>
          <Textarea
            dir="auto"
            lang={locale}
            rows={3}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            aria-label={t("translation")}
          />
          {tr?.issues.map((i, n) => (
            <p key={n} className={cn("text-xs", i.severity === "error" ? "text-destructive" : "text-warning-foreground")}>
              {i.message}
            </p>
          ))}
          <div className="flex items-center gap-2">
            <Button size="sm" disabled={save.isPending || value === (tr?.value ?? "")} onClick={() => save.mutate()}>
              {save.isPending && <Loader2 className="animate-spin" />}
              {save.isPending ? tc("saving") : tc("save")}
            </Button>
            <Button variant="ghost" size="sm" asChild>
              <Link href={`/app/${project.slug}/keys?q=${encodeURIComponent(row.key)}&locale=${locale}`}>
                <ExternalLink /> {t("openInEditor")}
              </Link>
            </Button>
          </div>
          <ReviewBlock
            slug={project.slug}
            keyName={row.key}
            locale={locale}
            sourceLocale={project.source_locale}
            hasTranslation={!!tr?.value}
          />
        </div>
      )}
    </div>
  );
}
