"use client";

import { useQuery } from "@tanstack/react-query";
import {
  CircleAlert,
  CircleCheck,
  ExternalLink,
  FlaskConical,
  Grid3x3,
  Info,
  Maximize2,
  Minimize2,
  Monitor,
  RotateCw,
  Smartphone,
} from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  CodeBlock,
  EmptyState,
  ErrorState,
  PageHeader,
  useLocaleName,
} from "@/components/common";
import { SegmentPreview } from "@/components/experiments/results/segment-preview";
import { KeySheet, type KeyTarget } from "@/components/key-sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Link } from "@/i18n/navigation";
import { api, ApiError, qk, type Project, type VisualIssue } from "@/lib/api";
import { useMediaQuery, useOrigin, useProject, useSlug } from "@/lib/hooks";
import { BRIDGE_SNIPPET } from "@/lib/preview-bridge";
import { cn } from "@/lib/utils";
import {
  buildUrl,
  issueLabel,
  KIND_STYLE,
  parseViewport,
  previewTarget,
} from "@/lib/visual";

const FIT = 0; // zoom value meaning "fit every visible column in the width"
const ZOOMS = [FIT, 0.2, 0.25, 0.33, 0.5, 0.75, 1];
const PSEUDO = "pseudo";
const PAD = 48; // horizontal padding of the frame strip (px-6 × 2)
const GAP = 16; // gap-4 between columns

export default function PreviewPage() {
  const project = useProject(useSlug());
  const bySegment = useSearchParams().has("xp"); // ?xp=<id>: experiment arms × languages
  if (project.isError)
    return (
      <ErrorState error={project.error} onRetry={() => project.refetch()} />
    );
  if (!project.data) return <Skeleton className="h-[70vh] rounded-xl" />;
  return bySegment ? (
    <SegmentPreview project={project.data} />
  ) : (
    <Matrix project={project.data} />
  );
}

function Matrix({ project: p }: { project: Project }) {
  const t = useTranslations("matrix");
  const format = useFormatter();
  const settings = useQuery({
    queryKey: qk.preview(p.slug),
    queryFn: () => api.preview(p.slug),
  });
  const report = useQuery({
    queryKey: qk.visualReport(p.slug, "latest"),
    queryFn: () =>
      api
        .visualReport(p.slug, "latest")
        .catch((e) =>
          e instanceof ApiError && e.status === 404 ? null : Promise.reject(e),
        ),
  });
  const target = previewTarget(settings.data, report.data ?? undefined);

  const [route, setRoute] = useState("/");
  const [draftRoute, setDraftRoute] = useState("/");
  const [device, setDevice] = useState<"desktop" | "mobile">("desktop");
  const [zoom, setZoom] = useState(FIT);
  const [overlay, setOverlay] = useState(true);
  const [pseudo, setPseudo] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [heights, setHeights] = useState<Record<string, number>>({});
  const [loaded, setLoaded] = useState<Record<string, string>>({}); // column → frame key that fired `load`
  const [bridged, setBridged] = useState(false); // at least one frame runs the preview bridge
  const [sheet, setSheet] = useState<KeyTarget | null>(null);
  const [stripW, setStripW] = useState(0);
  const frames = useRef<Record<string, HTMLIFrameElement | null>>({});
  const stripRef = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    const ro = new ResizeObserver(([e]) =>
      setStripW(Math.round(e.contentRect.width)),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const viewport = target.viewports[device];
  const { w, h } = parseViewport(viewport);
  const defaultHeight = h * 2;
  const columns = [
    p.source_locale,
    ...(pseudo ? [PSEUDO] : []),
    ...p.target_locales,
  ];
  // Phones always show one locale (picked in the toolbar); on larger screens `focus` does the same.
  const compact = useMediaQuery("(max-width: 767px)");
  const localeName = useLocaleName();
  const [solo, setSolo] = useState<string>();
  const [focus, setFocus] = useState<string>();
  const soloCol =
    solo && columns.includes(solo)
      ? solo
      : (p.target_locales[0] ?? p.source_locale);
  const focused = focus && columns.includes(focus) ? focus : undefined;
  const shown = compact ? [soloCol] : focused ? [focused] : columns;
  const avail =
    (stripW || (typeof window === "undefined" ? 1280 : window.innerWidth)) -
    PAD -
    GAP * (shown.length - 1);
  const fit = Math.max(0.15, Math.min(1, avail / (shown.length * w)));
  const z =
    zoom === FIT ? fit : shown.length === 1 ? Math.min(zoom, fit) : zoom;
  const colName = (c: string) =>
    c === PSEUDO ? t("pseudoColumn") : localeName(c);

  // Bridge messages from the previewed app (see lib/preview-bridge.ts): exact heights + pseudo-locale.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const col = Object.entries(frames.current).find(
        ([, f]) => f?.contentWindow === e.source,
      )?.[0];
      if (
        !col ||
        typeof e.data?.type !== "string" ||
        !e.data.type.startsWith("glot:")
      )
        return;
      setBridged(true);
      if (e.data.type === "glot:height" && typeof e.data.height === "number") {
        const height = Math.min(8000, Math.max(200, e.data.height));
        setHeights((cur) =>
          cur[col] === height ? cur : { ...cur, [col]: height },
        );
      } else if (e.data.type === "glot:ready" && col === PSEUDO) {
        (e.source as Window).postMessage(
          { type: "glot:pseudo", on: true },
          "*",
        );
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  const issues: Record<string, VisualIssue[]> = {};
  for (const i of report.data?.report.issues ?? []) {
    if (i.route === route && i.viewport === viewport)
      (issues[i.locale] ??= []).push(i);
  }

  if (settings.isPending || report.isPending)
    return <Skeleton className="h-[70vh] rounded-xl" />;
  if (settings.isError)
    return (
      <ErrorState error={settings.error} onRetry={() => settings.refetch()} />
    );

  if (!target.base) {
    return (
      <div className="mx-auto max-w-3xl">
        <PageHeader title={t("title")} subtitle={t("subtitle")} />
        <EmptyState
          icon={Grid3x3}
          title={t("noBase.title")}
          body={t("noBase.body")}
          action={
            <Button asChild>
              <Link href={`/app/${p.slug}/settings#preview`}>
                {t("noBase.cta")}
              </Link>
            </Button>
          }
        >
          <ol className="mt-6 max-w-md list-decimal space-y-1.5 ps-5 text-start text-sm text-muted-foreground">
            <li>{t("noBase.step1")}</li>
            <li>{t("noBase.step2")}</li>
            <li>{t("noBase.step3")}</li>
          </ol>
        </EmptyState>
      </div>
    );
  }

  const go = (r: string) => {
    const next = r.trim()
      ? r.trim().startsWith("/")
        ? r.trim()
        : `/${r.trim()}`
      : "/";
    setRoute(next);
    setDraftRoute(next);
    setHeights({});
  };
  const openIssue = (i: VisualIssue) =>
    setSheet({
      locale: i.locale,
      key: i.key,
      text: i.text,
      issue: `${t(`kinds.${i.kind}`)} · ${issueLabel(i)}`,
    });

  return (
    <div className="-mx-4 -my-6 flex h-[calc(100dvh-3rem)] flex-col sm:-mx-6 lg:-mx-8">
      <div className="space-y-2 border-b px-4 py-2.5 sm:px-6">
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1">
            <h1 className="text-base font-semibold">{t("title")}</h1>
            <p className="hidden text-xs text-pretty text-muted-foreground sm:block">
              {t("howTo")}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <Button variant="ghost" size="sm" asChild>
              <Link href={`/app/${p.slug}/preview?xp=`}>
                <FlaskConical aria-hidden />{" "}
                <span className="hidden lg:inline">{t("bySegment")}</span>
              </Link>
            </Button>
            <BridgeHelp />
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("reload")}
              title={t("reload")}
              onClick={() => setReloadKey((k) => k + 1)}
            >
              <RotateCw />
            </Button>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <form
            className="flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              go(draftRoute);
            }}
          >
            <Label htmlFor="matrix-route" className="text-sm">
              {t("route")}
            </Label>
            <div
              className="flex h-8 items-center rounded-md border bg-background ps-2 shadow-xs"
              dir="ltr"
            >
              <span
                className="max-w-40 truncate font-mono text-xs text-muted-foreground"
                title={target.base}
              >
                {target.base.replace(/^https?:\/\//, "").replace(/\/$/, "")}
              </span>
              <Input
                id="matrix-route"
                list="matrix-routes"
                value={draftRoute}
                onChange={(e) => setDraftRoute(e.target.value)}
                onBlur={() => draftRoute !== route && go(draftRoute)}
                className="h-7 w-36 border-0 px-1 font-mono text-xs shadow-none focus-visible:ring-0"
                placeholder={t("routePlaceholder")}
              />
            </div>
            <datalist id="matrix-routes">
              {target.routes.map((r) => (
                <option key={r} value={r} />
              ))}
            </datalist>
          </form>
          {compact && (
            <Select value={soloCol} onValueChange={setSolo}>
              <SelectTrigger size="sm" aria-label={t("locale")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {columns.map((c) => (
                  <SelectItem key={c} value={c}>
                    <span className="font-mono text-xs">
                      {c === PSEUDO ? "qps" : c}
                    </span>{" "}
                    <span className="capitalize">{colName(c)}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          <div
            role="group"
            aria-label={t("viewport")}
            className="flex rounded-lg border p-0.5"
          >
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
                    setZoom(FIT);
                    setHeights({});
                  }}
                >
                  <Icon /> {t(d)}
                </Button>
              );
            })}
          </div>
          <Select
            value={String(zoom)}
            onValueChange={(v) => setZoom(Number(v))}
          >
            <SelectTrigger size="sm" className="w-28" aria-label={t("zoom")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ZOOMS.map((v) => (
                <SelectItem key={v} value={String(v)}>
                  {v === FIT
                    ? t("fit")
                    : format.number(v, { style: "percent" })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Tooltip>
            <TooltipTrigger asChild>
              <label className="flex items-center gap-2 text-sm">
                <Switch
                  checked={overlay && !!report.data}
                  onCheckedChange={setOverlay}
                  size="sm"
                  disabled={!report.data}
                />
                {t("overlay")}
              </label>
            </TooltipTrigger>
            <TooltipContent className="max-w-xs">
              {report.data ? t("overlayHint") : t("overlayNone")}
            </TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <label className="flex items-center gap-2 text-sm">
                <Switch
                  checked={pseudo}
                  onCheckedChange={setPseudo}
                  size="sm"
                />
                {t("pseudo")}
              </label>
            </TooltipTrigger>
            <TooltipContent className="max-w-xs">
              {t("pseudoHint")}
            </TooltipContent>
          </Tooltip>
          {focused && !compact && (
            <Button
              variant="outline"
              size="sm"
              className="ms-auto"
              onClick={() => setFocus(undefined)}
            >
              <Minimize2 /> {t("unfocus")}
            </Button>
          )}
        </div>
        {target.routes.length > 1 && (
          <nav
            aria-label={t("knownRoutes")}
            className="-mb-0.5 flex gap-1 overflow-x-auto pb-0.5"
            dir="ltr"
          >
            {target.routes.map((r) => (
              <Button
                key={r}
                size="xs"
                variant={r === route ? "secondary" : "ghost"}
                aria-current={r === route ? "page" : undefined}
                className="shrink-0 font-mono"
                onClick={() => go(r)}
              >
                {r}
              </Button>
            ))}
          </nav>
        )}
      </div>

      {report.data && overlay && (
        <p className="border-b bg-muted/30 px-4 py-1.5 text-xs text-muted-foreground sm:px-6">
          {t("reportFrom", {
            date: format.dateTime(new Date(report.data.created_at), {
              dateStyle: "medium",
              timeStyle: "short",
            }),
            count: Object.values(issues).reduce((n, l) => n + l.length, 0),
          })}{" "}
          <Link
            href={`/app/${p.slug}/visual`}
            className="underline underline-offset-4 hover:text-foreground"
          >
            {t("openReport")}
          </Link>
        </p>
      )}
      {!bridged && <BlankHelp slug={p.slug} />}

      {/* One scroll container for every column: scrolling is synchronised by construction. */}
      <div ref={stripRef} className="min-h-0 flex-1 overflow-auto bg-muted/30">
        <div className="flex w-max gap-4 p-4 sm:px-6" dir="ltr">
          {shown.map((col) => {
            const loc = col === PSEUDO ? p.source_locale : col;
            const url = buildUrl(
              target.base,
              target.pattern,
              loc,
              p.source_locale,
              route,
              true,
            );
            const frameKey = `${url}:${reloadKey}`;
            const height = heights[col] ?? defaultHeight;
            const colIssues = col === PSEUDO ? [] : (issues[col] ?? []);
            const stats =
              col === PSEUDO || col === p.source_locale
                ? undefined
                : p.stats.locales[col];
            return (
              <section
                key={col}
                className="shrink-0"
                style={{ width: w * z }}
                aria-label={colName(col)}
              >
                <ColumnHeader
                  slug={p.slug}
                  locale={col}
                  name={colName(col)}
                  isSource={col === p.source_locale}
                  missing={
                    stats
                      ? Math.max(0, p.stats.keys - stats.translated)
                      : undefined
                  }
                  url={url}
                  issues={overlay ? colIssues : []}
                  onIssue={openIssue}
                  focused={!!focused}
                  onFocus={
                    compact || columns.length < 2
                      ? undefined
                      : () => setFocus(focused ? undefined : col)
                  }
                />
                <div
                  className="relative overflow-hidden rounded-lg border bg-background shadow-sm"
                  style={{ height: height * z }}
                >
                  {loaded[col] !== frameKey && (
                    <div
                      className="absolute inset-0 z-10 grid place-items-center bg-background"
                      role="status"
                    >
                      <span className="flex items-center gap-2 text-xs text-muted-foreground">
                        <RotateCw
                          className="size-3.5 animate-spin"
                          aria-hidden
                        />{" "}
                        {t("loading")}
                      </span>
                    </div>
                  )}
                  <div
                    className="absolute top-0 left-0 origin-top-left"
                    style={{ width: w, height, transform: `scale(${z})` }}
                  >
                    <iframe
                      key={frameKey}
                      ref={(el) => {
                        frames.current[col] = el;
                      }}
                      src={url}
                      title={t("frameTitle", { locale: colName(col) })}
                      className="size-full border-0 bg-white"
                      loading="lazy"
                      onLoad={() =>
                        setLoaded((cur) => ({ ...cur, [col]: frameKey }))
                      }
                    />
                    {overlay &&
                      colIssues
                        .filter((i) => i.rect)
                        .map((i, n) => (
                          <Tooltip key={n}>
                            <TooltipTrigger asChild>
                              <button
                                type="button"
                                aria-label={t("issueAt", {
                                  kind: t(`kinds.${i.kind}`),
                                  key: issueLabel(i),
                                })}
                                onClick={() => openIssue(i)}
                                className="absolute animate-in rounded-sm bg-destructive/10 outline-destructive transition-colors hover:bg-destructive/25 fade-in"
                                style={{
                                  left: i.rect!.x,
                                  top: i.rect!.y,
                                  width: Math.max(i.rect!.width, 8),
                                  height: Math.max(i.rect!.height, 8),
                                  outlineWidth: 2 / z,
                                  outlineStyle: "solid",
                                }}
                              />
                            </TooltipTrigger>
                            <TooltipContent className="max-w-xs">
                              <p className="font-medium">
                                {t(`kinds.${i.kind}`)}
                              </p>
                              <p className="font-mono opacity-80">
                                {issueLabel(i)}
                              </p>
                              <p dir="auto" className="mt-1 line-clamp-3">
                                {i.text}
                              </p>
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
  slug,
  locale,
  name,
  isSource,
  missing,
  url,
  issues,
  onIssue,
  focused,
  onFocus,
}: {
  slug: string;
  locale: string;
  name: string;
  isSource: boolean;
  missing?: number;
  url: string;
  issues: VisualIssue[];
  onIssue: (i: VisualIssue) => void;
  focused: boolean;
  onFocus?: () => void;
}) {
  const t = useTranslations("matrix");
  return (
    <div className="sticky top-0 z-20 -mx-1 mb-2 space-y-1 bg-muted/80 px-1 py-1.5 text-sm backdrop-blur">
      <div className="flex items-center gap-2">
        <span className="rounded border bg-background px-1.5 font-mono text-xs">
          {locale === PSEUDO ? "qps" : locale}
        </span>
        <span className="min-w-0 truncate font-medium capitalize" title={name}>
          {name}
        </span>
        <div className="ms-auto flex shrink-0 items-center">
          {onFocus && (
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={onFocus}
              aria-label={focused ? t("unfocus") : t("focus")}
              title={focused ? t("unfocus") : t("focus")}
            >
              {focused ? <Minimize2 /> : <Maximize2 />}
            </Button>
          )}
          <Button variant="ghost" size="icon-xs" asChild>
            <a
              href={url}
              target="_blank"
              rel="noreferrer"
              aria-label={t("openTab")}
              title={t("openTab")}
            >
              <ExternalLink />
            </a>
          </Button>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1 text-xs">
        {isSource ? (
          <span className="text-muted-foreground">{t("source")}</span>
        ) : locale === PSEUDO ? (
          <span className="text-muted-foreground">{t("pseudoTag")}</span>
        ) : missing ? (
          <Link
            href={`/app/${slug}/keys?locale=${locale}&status=missing`}
            className="rounded px-1 text-warning underline-offset-4 hover:underline"
            title={t("missingHint", { language: name })}
          >
            {t("missing", { count: missing })}
          </Link>
        ) : (
          missing === 0 && (
            <span className="flex items-center gap-1 text-success">
              <CircleCheck className="size-3.5" aria-hidden /> {t("complete")}
            </span>
          )
        )}
        {issues.length > 0 && (
          <Popover>
            <PopoverTrigger asChild>
              <Button
                variant="ghost"
                size="xs"
                className="h-5 px-1 text-destructive"
              >
                <CircleAlert /> {t("issues", { count: issues.length })}
              </Button>
            </PopoverTrigger>
            <PopoverContent
              align="start"
              className="max-h-80 w-80 overflow-y-auto p-1"
            >
              <ul>
                {issues.map((i, n) => (
                  <li key={n}>
                    <button
                      type="button"
                      onClick={() => onIssue(i)}
                      className="w-full rounded-md px-2 py-1.5 text-start hover:bg-accent"
                    >
                      <span
                        className={cn(
                          "me-2 rounded px-1 text-[11px] font-medium",
                          KIND_STYLE[i.kind],
                        )}
                      >
                        {t(`kinds.${i.kind}`)}
                      </span>
                      <span className="font-mono text-xs">{issueLabel(i)}</span>
                      <span
                        dir="auto"
                        className="mt-0.5 block truncate text-xs text-muted-foreground"
                      >
                        {i.text}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </PopoverContent>
          </Popover>
        )}
      </div>
    </div>
  );
}

/** Cross-origin frames can't tell us they were blocked (the browser still fires `load`), so explain it up front. */
function BlankHelp({ slug }: { slug: string }) {
  const t = useTranslations("matrix.blank");
  const origin = useOrigin();
  return (
    <details className="group border-b bg-warning/5 px-4 py-1.5 text-xs sm:px-6">
      <summary className="cursor-pointer text-muted-foreground select-none hover:text-foreground">
        {t("summary")}
      </summary>
      <div className="space-y-1.5 py-1.5 text-pretty">
        <p>
          {t("body")}{" "}
          <code
            dir="ltr"
            className="rounded bg-muted px-1.5 py-0.5 font-mono break-all select-all"
          >
            Content-Security-Policy: frame-ancestors &apos;self&apos; {origin}
          </code>
        </p>
        <p className="text-muted-foreground">
          {t("other")}{" "}
          <Link
            href={`/app/${slug}/settings#preview`}
            className="underline underline-offset-4 hover:text-foreground"
          >
            {t("settings")}
          </Link>
        </p>
      </div>
    </details>
  );
}

function BridgeHelp() {
  const t = useTranslations("matrix.bridge");
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("title")}
          title={t("title")}
        >
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
