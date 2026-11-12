"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowRight, ExternalLink, Loader2, Network, RefreshCw, Search } from "lucide-react";
import { useTranslations } from "next-intl";
import { useDeferredValue, useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, PageHeader, StatusBadge } from "@/components/common";
import {
  textGraphApi,
  textGraphKeys,
  type AnchorDetail,
  type LocaleState,
  type Occurrence,
  type SkipReason,
} from "@/components/text-graph/api";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { useSlug } from "@/lib/hooks";
import { cn } from "@/lib/utils";

const KNOWN_SURFACES = ["catalog", "connector", "edge"] as const;
const STATE_STYLE: Record<LocaleState["state"], string> = {
  consistent: "bg-success/12 text-success-foreground",
  partial: "bg-info/12 text-info-foreground",
  missing: "bg-muted text-muted-foreground",
  inconsistent: "bg-warning/15 text-warning-foreground",
  conflict: "bg-destructive/10 text-destructive",
};

function useSurfaceName() {
  const t = useTranslations("textGraph");
  // An imported surface is shown under the name its importer chose.
  return (s: string) => ((KNOWN_SURFACES as readonly string[]).includes(s) ? t(`surface.${s as (typeof KNOWN_SURFACES)[number]}`) : s);
}

export default function TextGraphPage() {
  const t = useTranslations("textGraph");
  const slug = useSlug();
  const qc = useQueryClient();
  const surfaceName = useSurfaceName();
  const [input, setInput] = useState("");
  const q = useDeferredValue(input.trim());
  const [selected, setSelected] = useState<string | null>(null);
  const overview = useQuery({ queryKey: textGraphKeys.overview(slug), queryFn: () => textGraphApi.overview(slug), enabled: !!slug });
  const results = useQuery({ queryKey: textGraphKeys.search(slug, q), queryFn: () => textGraphApi.search(slug, q), enabled: !!slug });
  const refreshAll = () => qc.invalidateQueries({ queryKey: textGraphKeys.all(slug) });

  const sync = useMutation({ mutationFn: () => textGraphApi.sync(slug), onSuccess: refreshAll });
  const propagate = useMutation({
    mutationFn: () => textGraphApi.propagate(slug),
    onSuccess: (r) => {
      toast.success(t("applied", { count: r.written }));
      refreshAll();
    },
  });

  if (overview.isError) return <ErrorState error={overview.error} onRetry={() => overview.refetch()} />;
  const o = overview.data;
  const unsettled = o ? o.inconsistent + o.conflicts : 0;

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <>
            <Button variant="outline" onClick={() => sync.mutate()} disabled={sync.isPending}>
              <RefreshCw className={cn(sync.isPending && "animate-spin")} /> {t("refresh")}
            </Button>
            {o && o.fillable > 0 && (
              <Button onClick={() => propagate.mutate()} disabled={propagate.isPending} title={t("applyHint")}>
                {propagate.isPending && <Loader2 className="animate-spin" />}
                {t("apply", { count: o.fillable })}
              </Button>
            )}
          </>
        }
      />
      {!o ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : o.anchors === 0 ? (
        <EmptyState icon={Network} title={t("empty.title")} body={t("empty.body")} />
      ) : (
        <>
          <dl className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
            {(
              [
                ["sentences", o.anchors, ""],
                ["places", o.occurrences, Object.entries(o.surfaces).map(([s, n]) => `${surfaceName(s)} ${n}`).join(" · ")],
                ["shared", o.shared, t("stats.crossSurface", { count: o.cross_surface })],
                ["unsettled", unsettled, ""],
              ] as const
            ).map(([k, n, hint]) => (
              <div key={k} className="rounded-xl border bg-card px-4 py-3 shadow-(--shadow-card)">
                <dt className="text-xs text-muted-foreground">{t(`stats.${k}`)}</dt>
                <dd className="mt-1 text-2xl font-semibold tabular-nums">{n}</dd>
                {hint && <dd className="mt-0.5 truncate text-xs text-muted-foreground">{hint}</dd>}
              </div>
            ))}
          </dl>

          {o.reworded.length > 0 && (
            <section className="mb-6 rounded-xl border bg-card p-4 shadow-(--shadow-card)">
              <h2 className="text-sm font-medium">{t("reworded.title")}</h2>
              <ul className="mt-2 space-y-1.5 text-sm">
                {o.reworded.slice(0, 5).map((r) => (
                  <li key={r.from.id + r.to.id}>
                    <button type="button" className="text-start hover:underline" onClick={() => setSelected(r.from.id)}>
                      {t("reworded.item", { from: r.from.text, to: r.to.text, moved: r.moved, remaining: r.remaining })}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}

          <div className="grid gap-6 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
            <div>
              <div className="relative mb-3">
                <Search aria-hidden className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  type="search"
                  className="ps-9"
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  placeholder={t("searchPlaceholder")}
                  aria-label={t("searchPlaceholder")}
                />
              </div>
              {!q && o.inconsistencies.length > 0 && (
                <section className="mb-4">
                  <h2 className="mb-1 text-sm font-medium">{t("unsettled.title")}</h2>
                  <p className="mb-2 text-xs text-muted-foreground">{t("unsettled.body")}</p>
                  <ul className="divide-y rounded-xl border bg-card shadow-(--shadow-card)">
                    {o.inconsistencies.map((i) => (
                      <li key={i.anchor_id + i.locale}>
                        <button
                          type="button"
                          onClick={() => setSelected(i.anchor_id)}
                          aria-current={selected === i.anchor_id}
                          className="w-full px-3 py-2 text-start hover:bg-accent/40 aria-[current=true]:bg-accent/60"
                        >
                          <div className="flex items-center gap-2">
                            <span className="min-w-0 flex-1 truncate text-sm">{i.text}</span>
                            <Badge variant="secondary" className={cn("border-0 font-mono text-[11px]", STATE_STYLE[i.state])}>
                              {i.locale}
                            </Badge>
                          </div>
                          <div className="mt-0.5 truncate text-xs text-muted-foreground" lang={i.locale} dir="auto">
                            {i.values.map((v) => v.value).join("  ≠  ")}
                          </div>
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
              {results.isPending ? (
                <Skeleton className="h-40 rounded-xl" />
              ) : results.isError ? (
                <ErrorState error={results.error} onRetry={() => results.refetch()} />
              ) : results.data.anchors.length === 0 ? (
                <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">{t("noResults", { q })}</p>
              ) : (
                <ul className="divide-y rounded-xl border bg-card shadow-(--shadow-card)">
                  {results.data.anchors.map((a) => (
                    <li key={a.id}>
                      <button
                        type="button"
                        onClick={() => setSelected(a.id)}
                        aria-current={selected === a.id}
                        className="w-full px-3 py-2 text-start hover:bg-accent/40 aria-[current=true]:bg-accent/60"
                      >
                        <div className="line-clamp-2 text-sm">{a.text}</div>
                        <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                          <span>{t("places", { count: a.occurrences })}</span>
                          {a.surfaces.map((s) => (
                            <Badge key={s} variant="outline" className="h-5 px-1.5 text-[11px]">
                              {surfaceName(s)}
                            </Badge>
                          ))}
                          {a.context === "action" && <Badge variant="secondary" className="h-5 px-1.5 text-[11px]">{t("context.action")}</Badge>}
                        </div>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div className="min-w-0">
              {selected ? (
                <AnchorView key={selected} slug={slug} id={selected} onOpen={setSelected} onChanged={refreshAll} />
              ) : (
                <p className="rounded-xl border border-dashed px-4 py-14 text-center text-sm text-muted-foreground">{t("pick")}</p>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function AnchorView({ slug, id, onOpen, onChanged }: { slug: string; id: string; onOpen: (id: string) => void; onChanged: () => void }) {
  const t = useTranslations("textGraph");
  const surfaceName = useSurfaceName();
  const detail = useQuery({ queryKey: textGraphKeys.anchor(slug, id), queryFn: () => textGraphApi.anchor(slug, id) });
  const [choice, setChoice] = useState<{ locale: string; value: string } | null>(null);

  if (detail.isPending) return <Skeleton className="h-96 rounded-xl" />;
  if (detail.isError) return <ErrorState error={detail.error} onRetry={() => detail.refetch()} />;
  const d = detail.data;

  return (
    <article className="space-y-5 rounded-xl border bg-card p-4 shadow-(--shadow-card) sm:p-5">
      <header>
        <h2 className="text-lg font-medium text-pretty">{d.text}</h2>
        {d.context === "action" && <p className="mt-1 text-xs text-muted-foreground">{t("context.actionHint")}</p>}
      </header>

      <section>
        <h3 className="mb-2 text-sm font-medium">{t("detail.places", { count: d.occurrences.length })}</h3>
        <ul className="divide-y rounded-lg border text-sm">
          {d.occurrences.map((o) => (
            <li key={o.surface + o.locator} className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2">
              <Badge variant="outline" className="h-5 px-1.5 text-[11px]">
                {surfaceName(o.surface)}
              </Badge>
              <PlaceName o={o} />
              {o.text !== d.text && <span className="basis-full text-xs text-muted-foreground">{o.text}</span>}
            </li>
          ))}
        </ul>
      </section>

      <section>
        <h3 className="mb-2 text-sm font-medium">{t("detail.translations")}</h3>
        <div className="space-y-3">
          {d.locales.map((l) => (
            <LocaleBlock key={l.locale} detail={d} l={l} onChoose={(value) => setChoice({ locale: l.locale, value })} />
          ))}
        </div>
      </section>

      {d.moved.length > 0 && (
        <section>
          <h3 className="mb-2 text-sm font-medium">{t("detail.moved")}</h3>
          <ul className="space-y-1 text-sm">
            {d.moved.map((m) => (
              <li key={m.surface + m.locator}>
                <button type="button" className="inline-flex items-center gap-1.5 text-start hover:underline" onClick={() => onOpen(m.to.id)}>
                  <span className="font-mono text-xs">{m.locator}</span>
                  <ArrowRight aria-hidden className="size-3.5 shrink-0 rtl:rotate-180" />
                  <span>{m.to.text}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {d.related.length > 0 && (
        <section>
          <h3 className="mb-2 text-sm font-medium">{t("detail.related")}</h3>
          <ul className="space-y-1 text-sm">
            {d.related.map((r) => (
              <li key={r.id}>
                <button type="button" className="flex w-full items-center gap-2 text-start hover:underline" onClick={() => onOpen(r.id)}>
                  <span className="min-w-0 flex-1 truncate">{r.text}</span>
                  <Badge variant="secondary" className="h-5 shrink-0 px-1.5 text-[11px]">
                    {t(`related.${r.kind}`)}
                  </Badge>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {choice && (
        <UnifyDialog slug={slug} id={id} locale={choice.locale} value={choice.value} onClose={() => setChoice(null)} onDone={onChanged} />
      )}
    </article>
  );
}

/** Where a human can find the place: the key, the page path, the CMS field. */
function PlaceName({ o }: { o: Occurrence }) {
  const t = useTranslations("textGraph");
  if (o.surface === "edge") {
    return (
      <>
        <span className="min-w-0 truncate font-mono text-xs">{o.meta.url ?? "/"}</span>
        {o.role && <span className="text-xs text-muted-foreground">&lt;{o.role}&gt;</span>}
        {typeof o.meta.hits === "number" && <span className="ms-auto text-xs text-muted-foreground tabular-nums">{t("seen", { count: o.meta.hits })}</span>}
      </>
    );
  }
  const url = typeof o.meta.url === "string" && /^https?:\/\//.test(o.meta.url) ? o.meta.url : null;
  return (
    <>
      <span className="min-w-0 truncate font-mono text-xs">{o.locator}</span>
      {o.meta.account && <span className="text-xs text-muted-foreground">{o.meta.account}</span>}
      {url && (
        <a href={url} target="_blank" rel="noreferrer" className="ms-auto text-muted-foreground hover:text-foreground" aria-label={t("open")}>
          <ExternalLink className="size-3.5" />
        </a>
      )}
    </>
  );
}

function LocaleBlock({ detail, l, onChoose }: { detail: AnchorDetail; l: LocaleState; onChoose: (value: string) => void }) {
  const t = useTranslations("textGraph");
  const [custom, setCustom] = useState("");
  const inherited = detail.occurrences.filter((o) => o.translations[l.locale]?.inherited).length;
  const settled = l.state === "consistent";
  return (
    <div className="rounded-lg border p-3">
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs">{l.locale}</span>
        <Badge variant="secondary" className={cn("h-5 border-0 px-1.5 text-[11px]", STATE_STYLE[l.state])}>
          {t(`state.${l.state}`, { count: l.missing })}
        </Badge>
        {inherited > 0 && <span className="text-xs text-muted-foreground">{t("inherited", { count: inherited })}</span>}
      </div>
      <ul className="space-y-1.5">
        {l.values.map((v) => (
          <li key={v.value} className="flex items-start gap-2 text-sm">
            <div className="min-w-0 flex-1">
              <div lang={l.locale} dir="auto">
                {v.value}
              </div>
              <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                <StatusBadge status={v.reviewed ? "reviewed" : "machine"} />
                {t("places", { count: v.count })}
              </div>
            </div>
            {!settled && (
              <Button size="sm" variant="outline" onClick={() => onChoose(v.value)}>
                {t("useEverywhere")}
              </Button>
            )}
          </li>
        ))}
      </ul>
      {l.blocked.length > 0 && (
        <ul className="mt-2 space-y-0.5 text-xs text-muted-foreground">
          {l.blocked.map((b) => (
            <li key={b.surface + b.locator}>
              <span className="font-mono">{b.locator}</span>: {t(`reason.${b.reason as SkipReason}`)}
              {b.detail ? ` (${b.detail})` : ""}
            </li>
          ))}
        </ul>
      )}
      <form
        className="mt-3 flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (custom.trim()) onChoose(custom.trim());
        }}
      >
        <Input value={custom} onChange={(e) => setCustom(e.target.value)} lang={l.locale} dir="auto" placeholder={t("custom")} aria-label={t("custom")} className="h-8 text-sm" />
        <Button type="submit" size="sm" variant="outline" disabled={!custom.trim()}>
          {t("useEverywhere")}
        </Button>
      </form>
    </div>
  );
}

/** Shows what the one action will do (a dry run) before doing it. */
function UnifyDialog({ slug, id, locale, value, onClose, onDone }: { slug: string; id: string; locale: string; value: string; onClose: () => void; onDone: () => void }) {
  const t = useTranslations("textGraph");
  const tc = useTranslations("common");
  const [force, setForce] = useState(false);
  const preview = useQuery({
    queryKey: [...textGraphKeys.anchor(slug, id), "preview", locale, value, force],
    queryFn: () => textGraphApi.unify(slug, id, { locale, value, force, dry_run: true }),
  });
  const run = useMutation({
    mutationFn: () => textGraphApi.unify(slug, id, { locale, value, force }),
    onSuccess: (r) => {
      toast.success(t("applied", { count: r.written }));
      onDone();
      onClose();
    },
  });
  const p = preview.data;
  const conflicts = p?.actions.filter((a) => a.reason === "reviewed_conflict").length ?? 0;
  const others = p?.actions.filter((a) => a.action === "skip" && a.reason !== "reviewed_conflict") ?? [];
  return (
    <AlertDialog open onOpenChange={(o) => !o && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("confirm.title")}</AlertDialogTitle>
          <AlertDialogDescription lang={locale} dir="auto" className="text-foreground">
            {value}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {preview.isError ? (
          <ErrorState error={preview.error} />
        ) : !p ? (
          <Skeleton className="h-16 rounded-lg" />
        ) : (
          <div className="space-y-2 text-sm">
            <p>{p.writes > 0 ? t("confirm.writes", { count: p.writes }) : t("confirm.none")}</p>
            {conflicts > 0 && <p>{t("confirm.conflicts", { count: conflicts })}</p>}
            {(conflicts > 0 || force) && (
              <label className="flex items-start gap-2">
                <Checkbox checked={force} onCheckedChange={(c) => setForce(c === true)} className="mt-0.5" />
                <span>{t("confirm.force")}</span>
              </label>
            )}
            {others.length > 0 && (
              <ul className="space-y-0.5 text-xs text-muted-foreground">
                {others.map((a) => (
                  <li key={a.surface + a.locator}>
                    <span className="font-mono">{a.locator}</span>: {t(`reason.${a.reason as SkipReason}`)}
                    {a.detail ? ` (${a.detail})` : ""}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={!p || p.writes === 0 || run.isPending}
            onClick={(e) => {
              e.preventDefault();
              run.mutate();
            }}
          >
            {run.isPending && <Loader2 className="animate-spin" />}
            {t("useEverywhere")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
