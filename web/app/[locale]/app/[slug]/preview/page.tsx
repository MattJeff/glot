"use client";

import { useQuery } from "@tanstack/react-query";
import { CircleAlert, ExternalLink, Grid3x3, Info, Monitor, RotateCw, Smartphone } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { CodeBlock, EmptyState, ErrorState, useLocaleName } from "@/components/common";
import { KeySheet, type KeyTarget } from "@/components/key-sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Link } from "@/i18n/navigation";
import { api, ApiError, qk, type Project, type VisualIssue } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";
import { BRIDGE_SNIPPET } from "@/lib/preview-bridge";
import { cn } from "@/lib/utils";
import { buildUrl, issueLabel, KIND_STYLE, parseViewport, previewTarget } from "@/lib/visual";

const ZOOMS = [0.2, 0.25, 0.33, 0.5, 0.75, 1];
const PSEUDO = "pseudo";

export default function PreviewPage() {
  const project = useProject(useSlug());
  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  return <Matrix project={project.data} />;
}

function Matrix({ project: p }: { project: Project }) {
  const t = useTranslations("matrix");
  const format = useFormatter();
  const settings = useQuery({ queryKey: qk.preview(p.slug), queryFn: () => api.preview(p.slug) });
  const report = useQuery({
    queryKey: qk.visualReport(p.slug, "latest"),
    queryFn: () => api.visualReport(p.slug, "latest").catch((e) => (e instanceof ApiError && e.status === 404 ? null : Promise.reject(e))),
  });
  const target = previewTarget(settings.data, report.data ?? undefined);

  const [route, setRoute] = useState("/");
  const [draftRoute, setDraftRoute] = useState("/");
  const [device, setDevice] = useState<"desktop" | "mobile">("desktop");
  const [zoom, setZoom] = useState(0.33);
  const [overlay, setOverlay] = useState(true);
  const [pseudo, setPseudo] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [heights, setHeights] = useState<Record<string, number>>({});
  const [sheet, setSheet] = useState<KeyTarget | null>(null);
  const frames = useRef<Record<string, HTMLIFrameElement | null>>({});

  const viewport = target.viewports[device];
  const { w, h } = parseViewport(viewport);
  const defaultHeight = h * 2;
  const columns = [p.source_locale, ...(pseudo ? [PSEUDO] : []), ...p.target_locales];

  // Bridge messages from the previewed app (see lib/preview-bridge.ts): exact heights + pseudo-locale.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const col = Object.entries(frames.current).find(([, f]) => f?.contentWindow === e.source)?.[0];
      if (!col || typeof e.data?.type !== "string") return;
      if (e.data.type === "glot:height" && typeof e.data.height === "number") {
        const height = Math.min(8000, Math.max(200, e.data.height));
        setHeights((cur) => (cur[col] === height ? cur : { ...cur, [col]: height }));
      } else if (e.data.type === "glot:ready" && col === PSEUDO) {
        (e.source as Window).postMessage({ type: "glot:pseudo", on: true }, "*");
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  const issues: Record<string, VisualIssue[]> = {};
  for (const i of report.data?.report.issues ?? []) {
    if (i.route === route && i.viewport === viewport) (issues[i.locale] ??= []).push(i);
  }

  if (settings.isPending || report.isPending) return <Skeleton className="h-[70vh] rounded-xl" />;
  if (settings.isError) return <ErrorState error={settings.error} onRetry={() => settings.refetch()} />;

  if (!target.base) {
    return (
      <div className="mx-auto max-w-3xl pt-10">
        <EmptyState
          icon={Grid3x3}
          title={t("noBase.title")}
          body={t("noBase.body")}
          action={
            <Button asChild>
              <Link href={`/app/${p.slug}/settings#preview`}>{t("noBase.cta")}</Link>
            </Button>
          }
        />
      </div>
    );
  }

  const go = (r: string) => {
    const next = r.trim() ? (r.trim().startsWith("/") ? r.trim() : `/${r.trim()}`) : "/";
    setRoute(next);
    setDraftRoute(next);
    setHeights({});
  };

  return (
    <div className="-mx-4 -my-6 flex h-[calc(100dvh-3rem)] flex-col sm:-mx-6 lg:-mx-8">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b px-4 py-2.5 sm:px-6">
        <h1 className="me-1 text-base font-semibold">{t("title")}</h1>
        <form
          className="flex items-center"
          onSubmit={(e) => {
            e.preventDefault();
            go(draftRoute);
          }}
        >
          <Label htmlFor="matrix-route" className="sr-only">
            {t("route")}
          </Label>
          <Input
            id="matrix-route"
            list="matrix-routes"
            dir="ltr"
            value={draftRoute}
            onChange={(e) => setDraftRoute(e.target.value)}
            onBlur={() => draftRoute !== route && go(draftRoute)}
            className="h-8 w-48 font-mono text-xs"
            placeholder={t("routePlaceholder")}
          />
          <datalist id="matrix-routes">
            {target.routes.map((r) => (
              <option key={r} value={r} />
            ))}
          </datalist>
        </form>
        <div role="group" aria-label={t("viewport")} className="flex rounded-lg border p-0.5">
          {(["desktop", "mobile"] as const).map((d) => {
            const Icon = d === "desktop" ? Monitor : Smartphone;
            return (
              <Button
                key={d}
                variant={device === d ? "secondary" : "ghost"}
                size="xs"
                aria-pressed={device === d}
                onClick={() => {
                  setDevice(d);
                  setZoom(d === "mobile" ? 0.75 : 0.33);
                  setHeights({});
                }}
              >
                <Icon /> {t(d)}
              </Button>
            );
          })}
        </div>
        <Select value={String(zoom)} onValueChange={(v) => setZoom(Number(v))}>
          <SelectTrigger size="sm" className="w-24" aria-label={t("zoom")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {ZOOMS.map((z) => (
              <SelectItem key={z} value={String(z)}>
                {format.number(z, { style: "percent" })}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={overlay} onCheckedChange={setOverlay} size="sm" disabled={!report.data} />
          {t("overlay")}
        </label>
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={pseudo} onCheckedChange={setPseudo} size="sm" />
          {t("pseudo")}
        </label>
        <div className="ms-auto flex items-center gap-1">
          <span className="hidden truncate font-mono text-xs text-muted-foreground md:inline" dir="ltr">
            {target.base}
          </span>
          <BridgeHelp />
          <Button variant="ghost" size="icon-sm" aria-label={t("reload")} onClick={() => setReloadKey((k) => k + 1)}>
            <RotateCw />
          </Button>
        </div>
      </div>

      {report.data && overlay && (
        <p className="border-b bg-muted/30 px-4 py-1.5 text-xs text-muted-foreground sm:px-6">
          {t("reportFrom", {
            date: format.dateTime(new Date(report.data.created_at), { dateStyle: "medium", timeStyle: "short" }),
            count: Object.values(issues).reduce((n, l) => n + l.length, 0),
          })}
        </p>
      )}

      {/* One scroll container for every column: scrolling is synchronised by construction. */}
      <div className="min-h-0 flex-1 overflow-auto bg-muted/30">
        <div className="flex w-max gap-4 p-4 sm:px-6" dir="ltr">
          {columns.map((col) => {
            const loc = col === PSEUDO ? p.source_locale : col;
            const url = buildUrl(target.base, target.pattern, loc, p.source_locale, route, true);
            const height = heights[col] ?? defaultHeight;
            const colIssues = col === PSEUDO ? [] : (issues[col] ?? []);
            return (
              <section key={col} className="shrink-0" style={{ width: w * zoom }} aria-label={col}>
                <ColumnHeader
                  locale={col}
                  sourceLocale={p.source_locale}
                  url={url}
                  issues={overlay ? colIssues : []}
                  onIssue={(i) => setSheet({ locale: i.locale, key: i.key, text: i.text, issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}` })}
                />
                <div
                  className="relative overflow-hidden rounded-lg border bg-background shadow-sm"
                  style={{ height: height * zoom }}
                >
                  <div className="absolute top-0 left-0 origin-top-left" style={{ width: w, height, transform: `scale(${zoom})` }}>
                    <iframe
                      key={`${url}:${reloadKey}`}
                      ref={(el) => {
                        frames.current[col] = el;
                      }}
                      src={url}
                      title={t("frameTitle", { locale: col })}
                      className="size-full border-0 bg-white"
                      loading="lazy"
                    />
                    {overlay &&
                      colIssues
                        .filter((i) => i.rect)
                        .map((i, n) => (
                          <Tooltip key={n}>
                            <TooltipTrigger asChild>
                              <button
                                type="button"
                                aria-label={t("issueAt", { kind: i.kind, key: issueLabel(i) })}
                                onClick={() => setSheet({ locale: i.locale, key: i.key, text: i.text, issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}` })}
                                className="absolute animate-in rounded-sm bg-destructive/10 outline-destructive transition-colors hover:bg-destructive/25 fade-in"
                                style={{
                                  left: i.rect!.x,
                                  top: i.rect!.y,
                                  width: Math.max(i.rect!.width, 8),
                                  height: Math.max(i.rect!.height, 8),
                                  outlineWidth: 2 / zoom,
                                  outlineStyle: "solid",
                                }}
                              />
                            </TooltipTrigger>
                            <TooltipContent className="max-w-xs">
                              <p className="font-medium">{t(`kinds.${i.kind}`)}</p>
                              <p className="font-mono opacity-80">{issueLabel(i)}</p>
                              <p dir="auto" className="mt-1 line-clamp-3">{i.text}</p>
                            </TooltipContent>
                          </Tooltip>
                        ))}
                  </div>
                </div>
              </section>
            );
          })}
        </div>
      </div>
      <KeySheet project={p} target={sheet} onClose={() => setSheet(null)} />
    </div>
  );
}

function ColumnHeader({
  locale,
  sourceLocale,
  url,
  issues,
  onIssue,
}: {
  locale: string;
  sourceLocale: string;
  url: string;
  issues: VisualIssue[];
  onIssue: (i: VisualIssue) => void;
}) {
  const t = useTranslations("matrix");
  const localeName = useLocaleName();
  return (
    <div className="sticky top-0 z-10 -mx-1 mb-2 flex items-center gap-2 bg-muted/80 px-1 py-1.5 text-sm backdrop-blur">
      <span className="rounded border bg-background px-1.5 font-mono text-xs">{locale === PSEUDO ? "qps" : locale}</span>
      <span className={cn("truncate text-muted-foreground", locale !== PSEUDO && "capitalize")}>
        {locale === PSEUDO ? t("pseudoColumn") : localeName(locale)}
        {locale === sourceLocale && ` · ${t("source")}`}
      </span>
      {issues.length > 0 && (
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="xs" className="text-destructive">
              <CircleAlert /> {issues.length}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="max-h-80 w-80 overflow-y-auto p-1">
            <ul>
              {issues.map((i, n) => (
                <li key={n}>
                  <button type="button" onClick={() => onIssue(i)} className="w-full rounded-md px-2 py-1.5 text-start hover:bg-accent">
                    <span className={cn("me-2 rounded px-1 text-[11px] font-medium", KIND_STYLE[i.kind])}>{t(`kinds.${i.kind}`)}</span>
                    <span className="font-mono text-xs">{issueLabel(i)}</span>
                    <span dir="auto" className="mt-0.5 block truncate text-xs text-muted-foreground">
                      {i.text}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </PopoverContent>
        </Popover>
      )}
      <Button variant="ghost" size="icon-xs" className="ms-auto" asChild>
        <a href={url} target="_blank" rel="noreferrer" aria-label={t("openTab")}>
          <ExternalLink />
        </a>
      </Button>
    </div>
  );
}

function BridgeHelp() {
  const t = useTranslations("matrix.bridge");
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={t("title")}>
          <Info />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 space-y-2">
        <p className="text-sm font-medium">{t("title")}</p>
        <p className="text-xs text-muted-foreground">{t("body")}</p>
        <CodeBlock code={BRIDGE_SNIPPET} className="max-h-40 overflow-y-auto" />
      </PopoverContent>
    </Popover>
  );
}
