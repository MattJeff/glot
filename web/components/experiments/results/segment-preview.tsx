"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ExternalLink, FlaskConical, Monitor, Smartphone, Trophy } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { EmptyState, ErrorState, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Link, useRouter } from "@/i18n/navigation";
import { api, ApiError, qk, type Project, type XpExperiment } from "@/lib/api";
import { cn } from "@/lib/utils";
import { buildUrl, parseViewport, previewTarget } from "@/lib/visual";
import { ArmSwatch, type CellView, cellViews, useArmLabel, useXpFormat } from "./shared";

/** The arm a segment actually gets for good: its personalization or its winner; null while it is still split. */
const servedArm = (c: CellView | undefined) => (c && (c.state === "personalized" || c.state === "winner") ? c.winner : null);

/** Official texts of the experiment's keys (arm 0 has no stored values: it *is* the catalog). */
function useOfficial(slug: string, exp: XpExperiment) {
  const keys = exp.keys.map((k) => k.key);
  let prefix = keys[0] ?? "";
  for (const k of keys) while (!k.startsWith(prefix)) prefix = prefix.slice(0, -1);
  const q = useQuery({
    queryKey: [...qk.keys(slug), "xp", prefix],
    queryFn: () => api.keys(slug, { q: prefix || undefined, per_page: 100 }),
  });
  return (key: string, locale: string) => {
    const row = q.data?.items.find((r) => r.key === key);
    return row && (row.translations[locale]?.value ?? row.source);
  };
}

/** Forces an arm in the previewed page without counting an exposure (`?glot_xp=<id>.<arm>`, see edge decide()). */
function forced(url: string, id: string, arm: number) {
  try {
    const u = new URL(url);
    u.searchParams.set("glot_xp", `${id}.${arm}`);
    return u.toString();
  } catch {
    return url;
  }
}

/** Text-only matrix: for one language, every arm's wording of every key. Works without a preview URL. */
export function WordingMatrix({ slug, exp, cell }: { slug: string; exp: XpExperiment; cell?: CellView }) {
  const t = useTranslations("experimentResults");
  const { prob } = useXpFormat();
  const official = useOfficial(slug, exp);
  const name = useLocaleName();
  const label = useArmLabel(exp);
  const served = servedArm(cell);
  return (
    <Tabs defaultValue={exp.locales[0]}>
      <TabsList className="h-auto max-w-full flex-wrap justify-start">
        {exp.locales.map((l) => (
          <TabsTrigger key={l} value={l} className="gap-1.5">
            {name(l)} <span className="font-mono text-xs text-muted-foreground">{l}</span>
          </TabsTrigger>
        ))}
      </TabsList>
      {exp.locales.map((l) => (
        <TabsContent key={l} value={l} className="mt-3">
          <ul className="grid gap-3 md:grid-cols-2">
            {exp.arms
              .filter((a) => a.approved)
              .map((a) => {
                const stats = cell?.arms.find((x) => x.arm === a.arm);
                return (
                  <li key={a.arm} className={cn("rounded-lg border p-3", served === a.arm && "border-success/50 bg-success/5")}>
                    <div className="flex flex-wrap items-center gap-2 text-sm">
                      <ArmSwatch arm={a.arm} />
                      <span className="font-medium">{label(a.arm)}</span>
                      {served === a.arm && (
                        <span className="inline-flex items-center gap-1 text-xs text-success-foreground">
                          <Trophy className="size-3" aria-hidden /> {t("served")}
                        </span>
                      )}
                      {stats && (
                        <span className="ms-auto text-xs text-muted-foreground tabular-nums">{t("pBestShort", { p: prob(stats.p_best) })}</span>
                      )}
                    </div>
                    <dl className="mt-2 space-y-1.5">
                      {exp.keys.map(({ key }) => (
                        <div key={key}>
                          <dt className="font-mono text-[11px] text-muted-foreground" dir="ltr" translate="no">
                            {key}
                          </dt>
                          <dd lang={l} dir="auto" className="text-sm text-pretty">
                            {(a.arm === 0 ? official(key, l) : a.values[l]?.[key]) ?? "—"}
                          </dd>
                        </div>
                      ))}
                    </dl>
                  </li>
                );
              })}
          </ul>
        </TabsContent>
      ))}
    </Tabs>
  );
}

/**
 * "By segment" mode of the locale matrix (/preview?xp=<id>[&cell=<id>]): rows = arms, columns = languages,
 * each frame is the real page with that arm forced. The selected segment marks the arm it is served.
 */
export function SegmentPreview({ project: p }: { project: Project }) {
  const t = useTranslations("experimentResults.preview");
  const { cellName } = useXpFormat();
  const name = useLocaleName();
  const router = useRouter();
  const params = useSearchParams();
  const list = useQuery({
    queryKey: qk.xpList(p.slug),
    queryFn: () => api.xpList(p.slug),
  });
  const settings = useQuery({
    queryKey: qk.preview(p.slug),
    queryFn: () => api.preview(p.slug),
  });
  const report = useQuery({
    queryKey: qk.visualReport(p.slug, "latest"),
    queryFn: () => api.visualReport(p.slug, "latest").catch((e) => (e instanceof ApiError && e.status === 404 ? null : Promise.reject(e))),
  });
  const [device, setDevice] = useState<"desktop" | "mobile">("desktop");
  const [route, setRoute] = useState("/");
  const [locale, setLocale] = useState("all");

  const xps = list.data ?? [];
  const exp = xps.find((e) => e.id === params.get("xp")) ?? xps[0];
  const results = useQuery({
    queryKey: qk.xpResults(p.slug, exp?.id ?? ""),
    queryFn: () => api.xpResults(p.slug, exp!.id).catch(() => null),
    enabled: !!exp?.started_at,
  });
  const cells = exp ? cellViews(exp, results.data) : [];
  const cellId = params.get("cell") ?? cells[0]?.id;
  const cell = cells.find((c) => c.id === cellId);
  const set = (k: string, v: string) => {
    const q = new URLSearchParams(params.toString());
    q.set(k, v);
    if (k === "xp") q.delete("cell");
    router.replace(`/app/${p.slug}/preview?${q}`);
  };

  if (list.isError) return <ErrorState error={list.error} onRetry={() => list.refetch()} />;
  if (!list.data || settings.isPending || report.isPending) return <Skeleton className="h-[70vh] rounded-xl" />;

  const back = (
    <Button variant="ghost" size="sm" asChild>
      <Link href={`/app/${p.slug}/preview`}>
        <ArrowLeft className="rtl:-scale-x-100" aria-hidden /> {t("byLocale")}
      </Link>
    </Button>
  );
  if (!exp)
    return (
      <div className="mx-auto max-w-3xl space-y-4">
        {back}
        <EmptyState
          icon={FlaskConical}
          title={t("none.title")}
          body={t("none.body")}
          action={
            <Button asChild>
              <Link href={`/app/${p.slug}/experiments/new`}>{t("none.cta")}</Link>
            </Button>
          }
        />
      </div>
    );

  const target = previewTarget(settings.data, report.data ?? undefined);
  const { w, h } = parseViewport(target.viewports[device]);
  const zoom = device === "mobile" ? 0.5 : 0.25;
  const locales = locale === "all" ? exp.locales : [locale];
  const served = servedArm(cell);

  return (
    <div className="-mx-4 -my-6 flex min-h-[calc(100dvh-3rem)] flex-col sm:-mx-6 lg:-mx-8">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b px-4 py-2.5 sm:px-6">
        {back}
        <h1 className="me-1 text-base font-semibold">{t("title")}</h1>
        <Select value={exp.id} onValueChange={(v) => set("xp", v)}>
          <SelectTrigger size="sm" className="w-56 max-w-full" aria-label={t("experiment")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {xps.map((e) => (
              <SelectItem key={e.id} value={e.id}>
                {e.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={cellId} onValueChange={(v) => set("cell", v)}>
          <SelectTrigger size="sm" className="w-44 max-w-full" aria-label={t("segment")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {cells.map((c) => (
              <SelectItem key={c.id} value={c.id}>
                {cellName(c, cells.length === 1)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {target.base && (
          <Select value={locale} onValueChange={setLocale}>
            <SelectTrigger size="sm" className="w-40 max-w-full" aria-label={t("language")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("allLanguages")}</SelectItem>
              {exp.locales.map((l) => (
                <SelectItem key={l} value={l}>
                  {name(l)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
        {target.base && (
          <div role="group" aria-label={t("viewport")} className="flex rounded-lg border p-0.5">
            {(["desktop", "mobile"] as const).map((d) => {
              const Icon = d === "desktop" ? Monitor : Smartphone;
              return (
                <Button key={d} variant={device === d ? "secondary" : "ghost"} size="xs" aria-pressed={device === d} onClick={() => setDevice(d)}>
                  <Icon aria-hidden /> {t(d)}
                </Button>
              );
            })}
          </div>
        )}
        {target.base && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const v = new FormData(e.currentTarget).get("route")?.toString().trim() || "/";
              setRoute(v.startsWith("/") ? v : `/${v}`);
            }}
          >
            <Label htmlFor="xp-route" className="sr-only">
              {t("route")}
            </Label>
            <Input id="xp-route" name="route" dir="ltr" defaultValue={route} className="h-8 w-40 font-mono text-xs" list="xp-routes" />
            <datalist id="xp-routes">
              {target.routes.map((r) => (
                <option key={r} value={r} />
              ))}
            </datalist>
          </form>
        )}
      </div>
      {target.base && <p className="border-b bg-muted/30 px-4 py-1.5 text-xs text-muted-foreground sm:px-6">{t("notCounted")}</p>}

      {!target.base ? (
        <div className="space-y-4 p-4 sm:px-6">
          <p className="text-sm text-muted-foreground">
            {t("noBase")}{" "}
            <Link href={`/app/${p.slug}/settings#preview`} className="underline underline-offset-4">
              {t("noBaseCta")}
            </Link>
          </p>
          <WordingMatrix slug={p.slug} exp={exp} cell={cell} />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto bg-muted/30">
          <div className="w-max space-y-6 p-4 sm:px-6">
            {exp.arms
              .filter((a) => a.approved)
              .map((a) => (
                <ArmRow key={a.arm} exp={exp} arm={a.arm} served={served === a.arm}>
                  {locales.map((l) => {
                    const url = forced(buildUrl(target.base, target.pattern, l, p.source_locale, route, true), exp.id, a.arm);
                    return (
                      <section key={l} className="shrink-0" style={{ width: w * zoom }} aria-label={name(l)}>
                        <div className="mb-1.5 flex items-center gap-2 text-xs">
                          <span className="rounded border bg-background px-1.5 font-mono">{l}</span>
                          <span className="truncate text-muted-foreground">{name(l)}</span>
                          <Button variant="ghost" size="icon-xs" className="ms-auto" asChild>
                            <a href={url} target="_blank" rel="noreferrer" aria-label={t("openTab", { locale: name(l) })}>
                              <ExternalLink aria-hidden />
                            </a>
                          </Button>
                        </div>
                        <div className="relative overflow-hidden rounded-lg border bg-background shadow-sm" style={{ height: h * zoom }}>
                          <iframe
                            src={url}
                            title={t("frame", { locale: name(l) })}
                            loading="lazy"
                            className="absolute top-0 left-0 origin-top-left border-0 bg-white"
                            style={{
                              width: w,
                              height: h,
                              transform: `scale(${zoom})`,
                            }}
                          />
                        </div>
                      </section>
                    );
                  })}
                </ArmRow>
              ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ArmRow({ exp, arm, served, children }: { exp: XpExperiment; arm: number; served: boolean; children: React.ReactNode }) {
  const t = useTranslations("experimentResults");
  const label = useArmLabel(exp);
  return (
    <div>
      <h2 className="sticky start-0 mb-2 flex w-fit items-center gap-2 text-sm font-medium">
        <ArmSwatch arm={arm} /> {label(arm)}
        {served && (
          <span className="inline-flex items-center gap-1 text-xs font-normal text-success-foreground">
            <Trophy className="size-3" aria-hidden /> {t("served")}
          </span>
        )}
      </h2>
      <div className="flex gap-4" dir="ltr">
        {children}
      </div>
    </div>
  );
}
