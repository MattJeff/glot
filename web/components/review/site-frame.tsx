"use client";

import {
  Crosshair,
  Image as ImageIcon,
  MonitorPlay,
  PlugZap,
  RotateCw,
  X,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import type { ReviewAnchor } from "@/lib/api";
import { cn } from "@/lib/utils";

type Box = { x: number; y: number; w: number; h: number };
export type Pin = {
  id: string;
  n: number;
  anchor: ReviewAnchor;
  resolved: boolean;
  label: string;
};
/** What a click in the frame picked: element selector + text + document-relative box. */
export type Picked = Pick<
  ReviewAnchor,
  "selector" | "text" | "rect" | "point" | "key" | "key_source" | "dom"
>;
/** A live text substitution shown in the frame (manual edit or AI rewording preview). */
export type LivePreview = {
  anchor: Pick<ReviewAnchor, "selector" | "text" | "key">;
  annotationId?: string;
  value: string;
} | null;
export type Shot = { src: string; width: number } | null;
/** pick = the page asks for a click and handles it (edit, annotate without the widget); annotate = the widget runs its own composer. */
export type FrameMode = "idle" | "pick" | "annotate";
/** widget = the glot widget (contract §5) draws its own pins; bridge = lib/preview-bridge.ts, pins drawn here. */
export type BridgeState = "loading" | "widget" | "bridge" | "unreachable";
export type FrameEvent =
  | { type: "created" | "selected"; id: string }
  | { type: "navigated"; route: string; locale: string }
  /** The widget loaded but its API call failed (401 = review token, 0 = network/CORS). */
  | { type: "error"; status: number };

const BRIDGE_TIMEOUT_MS = 6000;
const GUTTER = 16;

/**
 * The customer's site in an iframe, with annotation pins laid over it.
 * Positions come from the preview bridge (lib/preview-bridge.ts) running inside the site; when the site never
 * answers (bridge missing, or the page refuses to be framed), falls back to the last visual-QA screenshot.
 */
export function SiteFrame({
  url,
  viewport,
  pins,
  selected,
  mode,
  preview,
  shot,
  reloadKey,
  onPick,
  onCancelPick,
  onPin,
  onEvent,
  onState,
  onInstall,
  onReload,
  reason,
}: {
  url: string;
  viewport: { w: number; h: number };
  pins: Pin[];
  selected: string | null;
  mode: FrameMode;
  preview: LivePreview;
  shot: Shot;
  reloadKey: number;
  onPick: (a: Picked) => void;
  onCancelPick: () => void;
  onPin: (id: string) => void;
  onEvent: (e: FrameEvent) => void;
  onState: (s: BridgeState) => void;
  onInstall: () => void;
  onReload: () => void;
  /** Why the site doesn't connect (from the setup checks), shown in the banner. */
  reason?: string;
}) {
  const t = useTranslations("reviewPage.frame");
  const box = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  // Bridge state + pin rects belong to one page load (url × reload); a new load starts over at "loading".
  const load = `${url}:${reloadKey}`;
  const [conn, setConn] = useState<{ load: string; state: BridgeState }>({
    load,
    state: "loading",
  });
  const state: BridgeState = conn.load === load ? conn.state : "loading";
  const setState = (next: BridgeState | ((s: BridgeState) => BridgeState)) =>
    setConn((c) => {
      const cur = c.load === load ? c.state : "loading";
      return { load, state: typeof next === "function" ? next(cur) : next };
    });
  // Fallback: the last visual-QA screenshot is shown by default when the site can't be reached.
  const [preferLive, setPreferLive] = useState(false);
  const [rectsFor, setRectsFor] = useState<{
    load: string;
    rects: Record<
      string,
      { x: number; y: number; width: number; height: number } | null
    >;
  }>({ load, rects: {} });
  const rects = rectsFor.load === load ? rectsFor.rects : {};
  const picking = mode !== "idle";
  const live = state === "widget" || state === "bridge";
  const siteOrigin = (() => {
    try {
      return new URL(url).origin;
    } catch {
      return "*";
    }
  })();

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) =>
      setSize({ w: e.contentRect.width, h: e.contentRect.height }),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // A new URL means a new page: wait for the bridge again.
  useEffect(() => {
    const timer = setTimeout(
      () =>
        setConn((c) =>
          c.load !== load
            ? { load, state: "unreachable" }
            : c.state === "loading"
              ? { load, state: "unreachable" }
              : c,
        ),
      BRIDGE_TIMEOUT_MS,
    );
    return () => clearTimeout(timer);
  }, [load]);

  useEffect(() => onState(state), [state]); // eslint-disable-line react-hooks/exhaustive-deps
  const showShot = state === "unreachable" && !!shot && !preferLive;
  const post = (msg: object) =>
    frame.current?.contentWindow?.postMessage(msg, siteOrigin);
  const anchorsMsg = {
    type: "glot:anchors",
    anchors: pins.map((p) => ({
      id: p.id,
      key: p.anchor.key,
      selector: p.anchor.selector,
      text: p.anchor.text,
    })),
  };
  const anchorsKey = JSON.stringify(anchorsMsg);

  // Latest callbacks for the long-lived message listener.
  const cb = useRef({
    onPick,
    onCancelPick,
    onEvent,
    anchorsMsg,
    setState,
    setRects: (r: typeof rects) => setRectsFor({ load, rects: r }),
  });
  useEffect(() => {
    cb.current = {
      onPick,
      onCancelPick,
      onEvent,
      anchorsMsg,
      setState,
      setRects: (r: typeof rects) => setRectsFor({ load, rects: r }),
    };
  });
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (
        !frame.current ||
        e.source !== frame.current.contentWindow ||
        typeof e.data?.type !== "string"
      )
        return;
      if (siteOrigin !== "*" && e.origin !== siteOrigin) return;
      const d = e.data;
      switch (d.type) {
        case "glot:review:ready":
          cb.current.setState("widget");
          break;
        case "glot:ready":
          cb.current.setState((s) => (s === "widget" ? s : "bridge"));
          post(cb.current.anchorsMsg);
          break;
        case "glot:rects":
          if (d.rects) cb.current.setRects(d.rects);
          break;
        case "glot:picked":
        case "glot:review:picked": {
          // Bridge: viewport rect + scroll offsets. Widget (§5 v1.2): a ready-made Anchor.
          if (d.anchor) cb.current.onPick(d.anchor);
          else if (d.rect) {
            const sx = Number(d.sx) || 0,
              sy = Number(d.sy) || 0;
            cb.current.onPick({
              key: d.key ?? null,
              key_source: d.key ? "attribute" : null,
              selector: d.selector ?? "",
              text: d.text ?? "",
              rect: {
                x: d.rect.x + sx,
                y: d.rect.y + sy,
                w: d.rect.width,
                h: d.rect.height,
              },
            });
          }
          break;
        }
        case "glot:review:error":
          cb.current.setState("unreachable");
          cb.current.onEvent({ type: "error", status: Number(d.status) || 0 });
          break;
        case "glot:pick-cancel":
          cb.current.onCancelPick();
          break;
        case "glot:review:created":
        case "glot:review:selected":
          if (typeof d.annotation_id === "string")
            cb.current.onEvent({
              type: d.type === "glot:review:created" ? "created" : "selected",
              id: d.annotation_id,
            });
          break;
        case "glot:review:navigated":
          if (typeof d.route === "string")
            cb.current.onEvent({
              type: "navigated",
              route: d.route,
              locale: String(d.locale ?? ""),
            });
          break;
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [siteOrigin]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (state === "bridge") post(JSON.parse(anchorsKey));
  }, [anchorsKey, state]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (state === "bridge") post({ type: "glot:pick", on: picking });
    if (state === "widget")
      post({
        type: "glot:review:mode",
        annotate: mode === "annotate",
        pick: mode === "pick",
      });
  }, [mode, state]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!live) return;
    const a = preview?.anchor;
    // Both dialects: the bridge's glot:replace and the widget's glot:review:preview (contract §5 v1.2).
    post({
      type: "glot:replace",
      key: a?.key,
      selector: a?.selector,
      text: a?.text,
      value: preview ? preview.value : null,
    });
    post({
      type: "glot:review:preview",
      annotation_id: preview?.annotationId ?? null,
      anchor: a ?? null,
      value: preview ? preview.value : null,
    });
  }, [preview?.value, preview?.anchor.selector, preview?.anchor.text, live]); // eslint-disable-line react-hooks/exhaustive-deps
  const sel = pins.find((p) => p.id === selected);
  useEffect(() => {
    if (!sel) return;
    if (state === "bridge")
      post({
        type: "glot:scroll",
        key: sel.anchor.key,
        selector: sel.anchor.selector,
        text: sel.anchor.text,
      });
    if (state === "widget")
      post({ type: "glot:review:focus", annotation_id: sel.id });
  }, [selected, state]); // eslint-disable-line react-hooks/exhaustive-deps

  const availW = Math.max(0, size.w - GUTTER * 2);
  const availH = Math.max(0, size.h - GUTTER * 2);
  const scale = availW ? Math.min(1, availW / viewport.w) : 1;
  const frameH = availH / scale;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {state === "unreachable" && (
        <div
          role="status"
          className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b bg-warning/10 px-4 py-2.5 text-sm"
        >
          <PlugZap
            className="size-4 shrink-0 text-warning-foreground"
            aria-hidden
          />
          <p className="min-w-0 flex-1 text-pretty">
            <span className="font-medium">{t("unreachable.title")}</span>{" "}
            {reason ?? t("unreachable.body")}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={onInstall}>
              <PlugZap /> {t("install")}
            </Button>
            {shot && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => setPreferLive((v) => !v)}
              >
                {showShot ? <MonitorPlay /> : <ImageIcon />}{" "}
                {showShot ? t("showLive") : t("showShot")}
              </Button>
            )}
            <Button size="sm" variant="ghost" onClick={onReload}>
              <RotateCw /> {t("retry")}
            </Button>
          </div>
        </div>
      )}
      {picking && (
        <div
          role="status"
          className="flex items-center gap-2 border-b bg-brand/10 px-4 py-2 text-sm"
        >
          <Crosshair className="size-4 shrink-0 text-brand" aria-hidden />
          <p className="flex-1">
            {live || showShot
              ? t(mode === "annotate" ? "annotating" : "picking")
              : t("pickingBlind")}
          </p>
          <Button size="sm" variant="ghost" onClick={onCancelPick}>
            <X /> {t("cancel")}
          </Button>
        </div>
      )}
      <div
        ref={box}
        className={cn(
          "relative min-h-0 flex-1 bg-muted/40",
          showShot ? "overflow-auto" : "overflow-hidden",
        )}
      >
        {size.w > 0 && (
          <div
            dir="ltr"
            className={cn(
              "relative mx-auto my-4 overflow-hidden rounded-lg border bg-background shadow-sm",
              picking && "ring-2 ring-brand",
            )}
            style={{
              width: viewport.w * scale,
              height: showShot ? undefined : frameH * scale,
            }}
          >
            {showShot ? (
              <ShotView
                shot={shot!}
                width={viewport.w * scale}
                pins={pins}
                selected={selected}
                picking={picking}
                onPick={onPick}
                onPin={onPin}
              />
            ) : (
              <>
                <div
                  className="absolute top-0 left-0 origin-top-left"
                  style={{
                    width: viewport.w,
                    height: frameH,
                    transform: `scale(${scale})`,
                  }}
                >
                  <iframe
                    key={`${url}:${reloadKey}`}
                    ref={frame}
                    src={url}
                    title={t("title", { url })}
                    className="size-full border-0 bg-white"
                  />
                </div>
                {state === "bridge" &&
                  pins.map((p) => {
                    const r = rects[p.id];
                    if (!r || r.y + r.height < 0 || r.y > frameH) return null;
                    return (
                      <PinMark
                        key={p.id}
                        pin={p}
                        selected={p.id === selected}
                        rect={{
                          x: r.x * scale,
                          y: r.y * scale,
                          w: r.width * scale,
                          h: r.height * scale,
                        }}
                        onClick={() => onPin(p.id)}
                      />
                    );
                  })}
                {state === "loading" && (
                  <p className="pointer-events-none absolute inset-x-0 bottom-3 mx-auto w-fit rounded-full bg-background/90 px-3 py-1 text-xs text-muted-foreground shadow-sm">
                    {t("connecting")}
                  </p>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ShotView({
  shot,
  width,
  pins,
  selected,
  picking,
  onPick,
  onPin,
}: {
  shot: NonNullable<Shot>;
  width: number;
  pins: Pin[];
  selected: string | null;
  picking: boolean;
  onPick: (a: Picked) => void;
  onPin: (id: string) => void;
}) {
  const t = useTranslations("reviewPage.frame");
  const k = width / shot.width;
  return (
    <div className="relative">
      {/* eslint-disable-next-line @next/next/no-img-element -- authenticated API file, not optimisable */}
      <img src={shot.src} alt={t("shotAlt")} className="block w-full" />
      {picking && (
        <button
          type="button"
          aria-label={t("pickHere")}
          className="absolute inset-0 cursor-crosshair bg-brand/5"
          onClick={(e) => {
            const b = e.currentTarget.getBoundingClientRect();
            onPick({
              selector: "",
              text: "",
              rect: {
                x: (e.clientX - b.left) / k,
                y: (e.clientY - b.top) / k,
                w: 0,
                h: 0,
              },
            });
          }}
        />
      )}
      {pins.map((p) => {
        const r = p.anchor.rect;
        if (!r) return null;
        return (
          <PinMark
            key={p.id}
            pin={p}
            selected={p.id === selected}
            rect={{ x: r.x * k, y: r.y * k, w: r.w * k, h: r.h * k }}
            onClick={() => onPin(p.id)}
          />
        );
      })}
    </div>
  );
}

function PinMark({
  pin,
  rect,
  selected,
  onClick,
}: {
  pin: Pin;
  rect: Box;
  selected: boolean;
  onClick: () => void;
}) {
  const px = pin.anchor.point?.x ?? 0,
    py = pin.anchor.point?.y ?? 0;
  return (
    <>
      {selected && rect.w > 0 && (
        <span
          aria-hidden
          className="pointer-events-none absolute rounded-sm border-2 border-brand bg-brand/10"
          style={{
            left: rect.x - 2,
            top: rect.y - 2,
            width: rect.w + 4,
            height: rect.h + 4,
          }}
        />
      )}
      <button
        type="button"
        onClick={onClick}
        aria-label={pin.label}
        aria-pressed={selected}
        className={cn(
          "absolute grid size-7 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full border-2 border-background text-xs font-semibold tabular-nums shadow-md transition-transform duration-150 motion-safe:hover:scale-110 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:outline-none",
          pin.resolved
            ? "bg-muted text-muted-foreground"
            : "bg-brand text-brand-foreground",
          selected &&
            "z-10 motion-safe:scale-110 ring-2 ring-brand ring-offset-2 ring-offset-background",
        )}
        style={{
          left: Math.max(14, rect.x + rect.w * px),
          top: Math.max(14, rect.y + rect.h * py),
        }}
      >
        {pin.n}
      </button>
    </>
  );
}
