"use client";

import { useOrigin } from "@/lib/hooks";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Keyboard,
  Link2,
  MessageSquarePlus,
  Monitor,
  PanelRight,
  PenLine,
  RotateCw,
  Smartphone,
  Tablet,
  Globe,
} from "lucide-react";
import { useTranslations } from "next-intl";
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, useLocaleName } from "@/components/common";
import {
  ALL,
  AnnotationList,
  DEFAULT_FILTERS,
  isDone,
  matches,
  type Filters,
} from "@/components/review/panel";
import { diagnose, Setup, type SetupInput } from "@/components/review/setup";
import { ShareDialog } from "@/components/review/share-dialog";
import {
  SiteFrame,
  type BridgeState,
  type FrameEvent,
  type FrameMode,
  type LivePreview,
  type Picked,
  type Pin,
} from "@/components/review/site-frame";
import { type EditTarget, TextEditor } from "@/components/review/text-editor";
import { AnnotationDetail, Composer } from "@/components/review/thread";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Link } from "@/i18n/navigation";
import {
  api,
  ApiError,
  qk,
  type Annotation,
  type NewAnnotation,
  type Project,
  type ReviewEvent,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import { buildUrl, parseViewport, previewTarget } from "@/lib/visual";

const VIEWPORTS = {
  desktop: { w: 1280, h: 800 },
  tablet: { w: 820, h: 1180 },
  mobile: { w: 390, h: 844 },
} as const;
type Device = keyof typeof VIEWPORTS;
const DEVICE_ICON = {
  desktop: Monitor,
  tablet: Tablet,
  mobile: Smartphone,
} as const;
// ponytail: one page of 200, filtered client-side; server-side filters/pagination when projects exceed it.
const PER_PAGE = 200;
const WEEK_MS = 7 * 24 * 3600 * 1000;
const SHORTCUTS_KEY = "glot.review.shortcuts";

type View =
  | { kind: "list" }
  | { kind: "annotation"; id: string }
  | { kind: "compose"; picked: Picked }
  | { kind: "edit"; target: EditTarget };

const noop = () => () => {};
function useMedia(query: string) {
  return useSyncExternalStore(
    (cb) => {
      const m = window.matchMedia(query);
      m.addEventListener("change", cb);
      return () => m.removeEventListener("change", cb);
    },
    () => window.matchMedia(query).matches,
    () => true,
  );
}
function useShortcutsSetting() {
  const enabled = useSyncExternalStore(
    noop,
    () => {
      try {
        return localStorage.getItem(SHORTCUTS_KEY) !== "off";
      } catch {
        return true;
      }
    },
    () => true,
  );
  const [override, setOverride] = useState<boolean | null>(null);
  const set = (on: boolean) => {
    setOverride(on);
    try {
      localStorage.setItem(SHORTCUTS_KEY, on ? "on" : "off");
    } catch {}
  };
  return [override ?? enabled, set] as const;
}

const typing = (el: EventTarget | null) =>
  el instanceof HTMLElement &&
  (el.isContentEditable ||
    /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) ||
    !!el.closest("[role=dialog],[role=menu],[role=listbox]"));

/** "Annotations" tab: the customer's live site on the left, the review panel on the right (a drawer below lg). */
export function Workspace({
  project: p,
  initial,
}: {
  project: Project;
  initial?: string;
}) {
  const t = useTranslations("reviewPage");
  const ts = useTranslations("reviewPage.setup");
  const localeName = useLocaleName();
  const qc = useQueryClient();
  const slug = p.slug;
  const origin = useOrigin() ?? "";
  const isLg = useMedia("(min-width: 1024px)");
  const isSm = useMedia("(min-width: 640px)");
  const [shortcuts, setShortcuts] = useShortcutsSetting();

  const settings = useQuery({
    queryKey: qk.preview(slug),
    queryFn: () => api.preview(slug),
  });
  const report = useQuery({
    queryKey: qk.visualReport(slug, "latest"),
    queryFn: () =>
      api
        .visualReport(slug, "latest")
        .catch((e) =>
          e instanceof ApiError && e.status === 404 ? null : Promise.reject(e),
        ),
  });
  const list = useQuery({
    queryKey: qk.annotations(slug),
    queryFn: () => api.annotations(slug, { per_page: PER_PAGE }),
  });
  const members = useQuery({
    queryKey: qk.reviewMembers(slug),
    queryFn: () => api.reviewMembers(slug),
  });
  const me = useQuery({ queryKey: qk.me, queryFn: api.me });
  const feedback = useQuery({
    queryKey: qk.feedback(slug),
    queryFn: () => api.feedback(slug),
  });
  const probe = useQuery({
    queryKey: ["review-probe", slug],
    queryFn: () => api.reviewProbe(slug),
    enabled: !!settings.data?.base_url,
    staleTime: 60_000,
    retry: false,
  });
  const session = useQuery({
    queryKey: ["review-session", slug],
    queryFn: () => api.reviewSession(slug),
    staleTime: 11 * 3600_000,
    retry: false,
  });

  const target = previewTarget(settings.data, report.data ?? undefined);
  const locales = [p.source_locale, ...p.target_locales];
  const [route, setRoute] = useState("/");
  const [draftRoute, setDraftRoute] = useState("/");
  const [locale, setLocale] = useState(p.target_locales[0] ?? p.source_locale);
  const [device, setDevice] = useState<Device>("desktop");
  const [mode, setMode] = useState<FrameMode>("idle");
  const [pickFor, setPickFor] = useState<"annotate" | "edit">("annotate");
  const [frameState, setFrameState] = useState<BridgeState>("loading");
  const [view, setView] = useState<View>(
    initial ? { kind: "annotation", id: initial } : { kind: "list" },
  );
  const [filters, setFilters] = useState<Filters>(DEFAULT_FILTERS);
  const [preview, setPreview] = useState<LivePreview>(null);
  const [drawer, setDrawer] = useState(false);
  const [share, setShare] = useState(false);
  const [install, setInstall] = useState(false);
  const [help, setHelp] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [widgetError, setWidgetError] = useState<number | null>(null);
  const [announce, setAnnounce] = useState("");

  // Phones always see the mobile layout, full width (the viewport switcher is hidden there).
  const vp = isSm ? VIEWPORTS[device] : VIEWPORTS.mobile;
  const siteUrl = target.base
    ? buildUrl(
        target.base,
        target.pattern,
        locale,
        p.source_locale,
        route,
        true,
      )
    : "";
  const token = session.data?.token;
  const frameUrl =
    siteUrl && token
      ? `${siteUrl}${siteUrl.includes("?") ? "&" : "?"}glot_review=${encodeURIComponent(token)}`
      : siteUrl;

  const all = useMemo(() => list.data?.items ?? [], [list.data]);
  // Stable pin numbers: creation order.
  const numbers = useMemo(
    () =>
      new Map(
        [...all]
          .sort((a, b) => a.created_at.localeCompare(b.created_at))
          .map((a, i) => [a.id, i + 1]),
      ),
    [all],
  );
  const items = useMemo(
    () => all.filter((a) => matches(a, filters)),
    [all, filters],
  );
  const [now] = useState(Date.now);
  const resolvedThisWeek = all.filter(
    (a) =>
      a.status === "resolved" &&
      a.resolved_at &&
      now - new Date(a.resolved_at).getTime() < WEEK_MS,
  ).length;
  const selected = view.kind === "annotation" ? view.id : null;
  const pins: Pin[] = all
    .filter(
      (a) =>
        a.route === route &&
        a.locale === locale &&
        (matches(a, { ...filters, route: ALL, locale: ALL }) ||
          a.id === selected),
    )
    .map((a) => ({
      id: a.id,
      n: numbers.get(a.id) ?? 0,
      anchor: a.anchor,
      resolved: isDone(a.status),
      label: t("pinLabel", { n: numbers.get(a.id) ?? 0, body: a.body }),
    }));

  const shotPage = report.data?.report.pages
    .filter((pg) => pg.locale === locale && pg.route === route)
    .sort(
      (a, b) =>
        Math.abs(parseViewport(a.viewport).w - vp.w) -
        Math.abs(parseViewport(b.viewport).w - vp.w),
    )[0];
  const shot =
    shotPage && report.data
      ? {
          src: api.visualFileUrl(slug, report.data.id, shotPage.screenshot),
          width: parseViewport(shotPage.viewport).w,
        }
      : null;

  const go = (r: string) => {
    const next = r.trim()
      ? r.trim().startsWith("/")
        ? r.trim()
        : `/${r.trim()}`
      : "/";
    setRoute(next);
    setDraftRoute(next);
  };
  const select = useCallback(
    (id: string) => {
      const a = all.find((x) => x.id === id);
      if (a) {
        if (a.route !== route) go(a.route);
        if (a.locale !== locale && locales.includes(a.locale))
          setLocale(a.locale);
      }
      setMode("idle");
      setView({ kind: "annotation", id });
      setDrawer(true);
    },
    [all, route, locale], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const back = () => {
    setView({ kind: "list" });
    setPreview(null);
  };
  const startAnnotate = () => {
    setPickFor("annotate");
    setMode(frameState === "widget" ? "annotate" : "pick");
    setDrawer(false);
  };
  const startEdit = () => {
    const a = selected ? all.find((x) => x.id === selected) : null;
    if (a) return editFor(a);
    setPickFor("edit");
    setMode("pick");
    setDrawer(false);
  };
  const editFor = (a: Annotation) => {
    setView({
      kind: "edit",
      target: {
        key: a.key,
        text: a.translation?.value ?? a.anchor.text,
        anchor: a.anchor,
      },
    });
    setDrawer(true);
  };

  const create = useMutation({
    mutationFn: (b: NewAnnotation) => api.createAnnotation(slug, b),
    onSuccess: (a) => {
      qc.setQueryData(
        qk.annotations(slug),
        (old: typeof list.data) =>
          old && { ...old, items: [a, ...old.items], total: old.total + 1 },
      );
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
      toast.success(t("created"));
      setView({ kind: "annotation", id: a.id });
    },
  });
  const resolve = useMutation({
    mutationFn: (id: string) =>
      api.updateAnnotation(slug, id, { status: "resolved" }),
    onSuccess: (a) => {
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
      qc.invalidateQueries({ queryKey: qk.annotation(slug, a.id) });
      toast.success(t("resolved", { n: numbers.get(a.id) ?? 0 }));
    },
  });

  const onPick = (picked: Picked) => {
    setMode("idle");
    setDrawer(true);
    if (pickFor === "edit")
      setView({
        kind: "edit",
        target: {
          key: picked.key ?? null,
          text: picked.text,
          anchor: { ...picked, url: siteUrl, locale, viewport: vp },
        },
      });
    else setView({ kind: "compose", picked });
  };
  const onFrameEvent = (e: FrameEvent) => {
    if (e.type === "error") return setWidgetError(e.status);
    if (e.type === "navigated") {
      if (e.route !== route) go(e.route);
      if (e.locale && e.locale !== locale && locales.includes(e.locale))
        setLocale(e.locale);
      return;
    }
    if (e.type === "created") {
      setMode("idle");
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
    }
    setView({ kind: "annotation", id: e.id });
    setDrawer(true);
  };

  const reload = () => {
    setWidgetError(null);
    setReloadKey((k) => k + 1);
  };
  const recheck = () => {
    probe.refetch();
    reload();
  };

  // Realtime (contract §4): refetch what an event touches; announce teammates' new annotations.
  const meId = me.data?.id;
  useEffect(() => {
    const es = new EventSource(api.reviewEventsUrl(slug));
    es.addEventListener("review", (m) => {
      let ev: ReviewEvent;
      try {
        ev = JSON.parse((m as MessageEvent).data);
      } catch {
        return;
      }
      qc.invalidateQueries({ queryKey: qk.annotations(slug), exact: true });
      if (ev.annotation_id)
        qc.invalidateQueries({
          queryKey: qk.annotation(slug, ev.annotation_id),
        });
      if (ev.kind.startsWith("text.") && typeof ev.data.key === "string")
        qc.invalidateQueries({ queryKey: qk.text(slug, ev.data.key) });
      if (
        ev.actor.user_id !== meId &&
        (ev.kind === "annotation.created" || ev.kind === "comment.created")
      )
        setAnnounce(
          t(
            ev.kind === "annotation.created"
              ? "live.annotation"
              : "live.comment",
            { name: ev.actor.name },
          ),
        );
    });
    return () => es.close();
  }, [slug, meId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keyboard: C comment, E edit text, J/K next/previous, R resolve, Esc cancel, ? help. Can be turned off (WCAG 2.1.4).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && mode !== "idle") {
        setMode("idle");
        return;
      }
      if (!shortcuts || e.metaKey || e.ctrlKey || e.altKey || typing(e.target))
        return;
      const k = e.key.toLowerCase();
      if (k === "c") startAnnotate();
      else if (k === "e") startEdit();
      else if (k === "j" || k === "k") {
        if (!items.length) return;
        const i = items.findIndex((a) => a.id === selected);
        const next =
          items[
            i < 0 ? 0 : (i + (k === "j" ? 1 : -1) + items.length) % items.length
          ];
        select(next.id);
        document
          .querySelector<HTMLElement>(`[data-annotation="${next.id}"]`)
          ?.scrollIntoView({ block: "nearest" });
      } else if (k === "r" && selected) resolve.mutate(selected);
      else if (e.key === "?") setHelp(true);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (settings.isPending || report.isPending)
    return <Skeleton className="m-4 h-[60vh] rounded-xl" />;
  if (!target.base)
    return (
      <div className="mx-auto w-full max-w-3xl overflow-y-auto p-4 sm:p-6">
        <EmptyState
          icon={Globe}
          title={t("noSite.title")}
          body={t("noSite.body")}
          action={
            <Button asChild>
              <Link href={`/app/${slug}/settings#preview`}>
                {t("noSite.cta")}
              </Link>
            </Button>
          }
        />
      </div>
    );

  // Review needs the review widget (pins, threads, frame bridge), served by glot itself: the feedback widget doesn't answer the frame.
  const widgetSnippet = `<script src="${origin}/widget/review.global.js" data-project="${slug}" data-endpoint="${origin}" data-app-origin="${origin}" async></script>`;
  const setupInput: SetupInput = {
    slug,
    origin,
    probe: probe.data,
    probing: probe.isFetching,
    frame: frameState,
    widgetError,
  };
  const diag = diagnose(setupInput);
  const live = frameState === "widget" || frameState === "bridge";
  const setup = (
    <Setup
      input={setupInput}
      siteUrl={target.base}
      snippet={widgetSnippet}
      onRecheck={recheck}
    />
  );
  const canPr =
    !!feedback.data?.settings.github_repo &&
    !!feedback.data?.settings.has_github_token;
  const onEditPreview = (key: string, value: string | null) => {
    const tg = view.kind === "edit" ? view.target : null;
    setPreview(
      value === null
        ? null
        : {
            anchor: {
              key,
              selector: tg?.anchor?.selector ?? "",
              text: tg?.text ?? "",
            },
            value,
          },
    );
  };

  const panel = (
    <>
      {view.kind === "list" && list.isError && (
        <div className="p-4">
          <ErrorState error={list.error} onRetry={() => list.refetch()} />
        </div>
      )}
      {view.kind === "list" && list.isPending && (
        <div className="space-y-3 p-4">
          <Skeleton className="h-8" />
          <Skeleton className="h-20" />
          <Skeleton className="h-20" />
        </div>
      )}
      {view.kind === "list" && list.isSuccess && (
        <AnnotationList
          items={items}
          all={all}
          numbers={numbers}
          filters={filters}
          onFilters={setFilters}
          selected={selected}
          onSelect={select}
          resolvedThisWeek={resolvedThisWeek}
          emptySetup={live ? null : setup}
          emptyAction={
            <Button onClick={startAnnotate}>
              <MessageSquarePlus /> {t("toolbar.comment")}
            </Button>
          }
        />
      )}
      {view.kind === "annotation" && (
        <AnnotationDetail
          key={view.id}
          slug={slug}
          id={view.id}
          n={numbers.get(view.id) ?? 0}
          meId={meId}
          members={members.data ?? []}
          canPr={canPr}
          onBack={back}
          onEditText={editFor}
          onPreview={(a, value) => {
            if (a.locale === locale && a.route === route)
              setPreview(
                value === null
                  ? null
                  : { anchor: a.anchor, annotationId: a.id, value },
              );
          }}
          onDeleted={back}
        />
      )}
      {view.kind === "compose" && (
        <Composer
          text={view.picked.text}
          annotationKey={view.picked.key ?? null}
          pending={create.isPending}
          onCancel={back}
          onSubmit={(b) =>
            create.mutate({
              ...b,
              anchor: { ...view.picked, url: siteUrl, locale, viewport: vp },
            })
          }
        />
      )}
      {view.kind === "edit" && (
        <TextEditor
          key={view.target.key ?? view.target.text}
          slug={slug}
          target={view.target}
          frameLocale={locale}
          onPreview={onEditPreview}
          onShowLocale={setLocale}
          onBack={back}
        />
      )}
    </>
  );

  const openCount = all.filter((a) => !isDone(a.status)).length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <p className="sr-only" role="status" aria-live="polite">
        {announce}
      </p>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-2 border-b px-4 py-2 sm:px-6">
        <form
          className="flex items-center"
          onSubmit={(e) => {
            e.preventDefault();
            go(draftRoute);
          }}
        >
          <Label htmlFor="rv-route" className="sr-only">
            {t("toolbar.route")}
          </Label>
          <Input
            id="rv-route"
            list="rv-routes"
            dir="ltr"
            value={draftRoute}
            onChange={(e) => setDraftRoute(e.target.value)}
            onBlur={() => draftRoute !== route && go(draftRoute)}
            className="h-8 w-32 font-mono text-xs sm:w-44"
            placeholder="/"
          />
          <datalist id="rv-routes">
            {[...new Set([...target.routes, ...all.map((a) => a.route)])].map(
              (r) => (
                <option key={r} value={r} />
              ),
            )}
          </datalist>
        </form>
        <Select value={locale} onValueChange={setLocale}>
          <SelectTrigger
            size="sm"
            className="h-8 w-auto"
            aria-label={t("toolbar.locale")}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {locales.map((l) => (
              <SelectItem key={l} value={l}>
                {localeName(l)}{" "}
                <span className="font-mono text-xs text-muted-foreground">
                  {l}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div
          role="group"
          aria-label={t("toolbar.viewport")}
          className="hidden rounded-lg border p-0.5 sm:flex"
        >
          {(Object.keys(VIEWPORTS) as Device[]).map((d) => {
            const Icon = DEVICE_ICON[d];
            return (
              <Button
                key={d}
                variant={device === d ? "secondary" : "ghost"}
                size="icon-sm"
                aria-pressed={device === d}
                aria-label={t(`toolbar.${d}`)}
                title={t(`toolbar.${d}`)}
                onClick={() => setDevice(d)}
              >
                <Icon />
              </Button>
            );
          })}
        </div>
        <Button
          size="sm"
          variant="ghost"
          className="h-8 gap-1.5 px-2 text-xs"
          onClick={() => setInstall(true)}
          aria-haspopup="dialog"
        >
          <span
            aria-hidden
            className={cn(
              "size-2 rounded-full",
              live
                ? "bg-success"
                : frameState === "loading"
                  ? "animate-pulse bg-muted-foreground"
                  : "bg-destructive",
            )}
          />
          {ts(`chip.${frameState}`)}
        </Button>
        <div className="ms-auto flex flex-wrap items-center gap-1.5">
          <Button
            size="sm"
            variant={
              mode !== "idle" && pickFor === "annotate"
                ? "secondary"
                : "default"
            }
            aria-pressed={mode !== "idle" && pickFor === "annotate"}
            onClick={startAnnotate}
          >
            <MessageSquarePlus />{" "}
            <span className="max-sm:sr-only">{t("toolbar.comment")}</span>
            <kbd className="hidden rounded border border-current/30 px-1 font-mono text-[10px] opacity-70 md:inline">
              C
            </kbd>
          </Button>
          <Button
            size="sm"
            variant="outline"
            aria-pressed={mode === "pick" && pickFor === "edit"}
            onClick={startEdit}
          >
            <PenLine />{" "}
            <span className="max-sm:sr-only">{t("toolbar.edit")}</span>
            <kbd className="hidden rounded border px-1 font-mono text-[10px] text-muted-foreground md:inline">
              E
            </kbd>
          </Button>
          <Button size="sm" variant="outline" onClick={() => setShare(true)}>
            <Link2 />{" "}
            <span className="max-sm:sr-only">{t("toolbar.share")}</span>
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("toolbar.shortcuts")}
            title={`${t("toolbar.shortcuts")} (?)`}
            onClick={() => setHelp(true)}
            className="max-sm:hidden"
          >
            <Keyboard />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("toolbar.reload")}
            title={t("toolbar.reload")}
            onClick={reload}
          >
            <RotateCw />
          </Button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="relative flex min-w-0 flex-1 flex-col">
          <SiteFrame
            url={frameUrl}
            viewport={vp}
            pins={pins}
            selected={selected}
            mode={mode}
            preview={preview}
            shot={shot}
            reloadKey={reloadKey}
            onPick={onPick}
            onCancelPick={() => setMode("idle")}
            onPin={select}
            onEvent={onFrameEvent}
            onState={setFrameState}
            onInstall={() => setInstall(true)}
            onReload={reload}
            reason={diag.reason && ts(diag.reason.msg, diag.reason.values)}
          />
          {!isLg && (
            <Button
              className="fixed end-4 bottom-4 z-20 shadow-lg"
              onClick={() => setDrawer(true)}
              aria-haspopup="dialog"
            >
              <PanelRight /> {t("openPanel", { count: openCount })}
            </Button>
          )}
        </div>
        {isLg ? (
          <aside
            aria-label={t("panel")}
            className={cn(
              "flex w-[380px] shrink-0 flex-col border-s bg-background",
              view.kind === "edit" && "xl:w-[480px]",
            )}
          >
            {panel}
          </aside>
        ) : (
          <Sheet open={drawer} onOpenChange={setDrawer}>
            <SheetContent
              side="right"
              className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-md"
            >
              <SheetHeader className="border-b p-3">
                <SheetTitle>{t("panel")}</SheetTitle>
                <SheetDescription className="sr-only">
                  {t("panelHint")}
                </SheetDescription>
              </SheetHeader>
              <div className="flex min-h-0 flex-1 flex-col">{panel}</div>
            </SheetContent>
          </Sheet>
        )}
      </div>

      <ShareDialog
        slug={slug}
        siteUrl={siteUrl}
        open={share}
        onOpenChange={setShare}
      />
      <Dialog open={install} onOpenChange={setInstall}>
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader className="sr-only">
            <DialogTitle>{t("setup.title")}</DialogTitle>
            <DialogDescription>{t("setup.body")}</DialogDescription>
          </DialogHeader>
          {setup}
        </DialogContent>
      </Dialog>
      <Dialog open={help} onOpenChange={setHelp}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("help.title")}</DialogTitle>
            <DialogDescription>{t("help.body")}</DialogDescription>
          </DialogHeader>
          <dl className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-sm">
            {(["c", "e", "j", "k", "r", "esc", "?"] as const).map((k) => (
              <div key={k} className="contents">
                <dt>
                  <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono text-xs uppercase">
                    {k}
                  </kbd>
                </dt>
                <dd>{t(`help.${k === "?" ? "help" : k}`)}</dd>
              </div>
            ))}
          </dl>
          <label className="flex items-center justify-between gap-3 border-t pt-3 text-sm">
            {t("help.enabled")}
            <Switch checked={shortcuts} onCheckedChange={setShortcuts} />
          </label>
        </DialogContent>
      </Dialog>
    </div>
  );
}
