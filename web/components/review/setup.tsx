"use client";

import {
  CircleAlert,
  CircleCheck,
  CircleHelp,
  Loader2,
  RotateCw,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { useId } from "react";
import { CodeBlock } from "@/components/common";
import type { BridgeState } from "@/components/review/site-frame";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { ReviewProbe } from "@/lib/api";
import { BRIDGE_SNIPPET } from "@/lib/preview-bridge";
import { cn } from "@/lib/utils";

type Status = "ok" | "fail" | "unknown" | "pending";
type SetupKey = Parameters<
  ReturnType<typeof useTranslations<"reviewPage.setup">>
>[0];
type Check = { status: Status; msg: SetupKey; values?: Record<string, string> };
export type SetupInput = {
  slug: string;
  origin: string;
  probe: ReviewProbe | undefined;
  probing: boolean;
  frame: BridgeState;
  /** Status posted by the widget when its API call failed (glot:review:error). */
  widgetError: number | null;
};

/** The three live checks (framing, widget, connection) and, when the frame is down, the most likely reason. */
export function diagnose({
  slug,
  origin,
  probe: p,
  probing,
  frame,
  widgetError,
}: SetupInput) {
  const live = frame === "widget" || frame === "bridge";
  // CSP frame-ancestors wins over X-Frame-Options when both are sent.
  const header = p?.frame_ancestors
    ? `Content-Security-Policy: frame-ancestors ${p.frame_ancestors}`
    : `X-Frame-Options: ${p?.x_frame_options ?? ""}`;
  const framing: Check = live
    ? { status: "ok", msg: "framing.ok" }
    : !p
      ? { status: probing ? "pending" : "unknown", msg: "framing.unchecked" }
      : p.error
        ? {
            status: "unknown",
            msg:
              p.error === "private" ? "framing.private" : "framing.unreachable",
          }
        : p.framing === "blocked"
          ? { status: "fail", msg: "framing.blocked", values: { header } }
          : { status: "ok", msg: "framing.ok" };
  const widget: Check =
    frame === "widget"
      ? { status: "ok", msg: "widget.ok" }
      : widgetError !== null
        ? {
            status: "fail",
            msg: widgetError === 401 ? "widget.denied" : "widget.network",
          }
        : frame === "bridge"
          ? { status: "ok", msg: "widget.bridge" }
          : !p || p.error
            ? {
                status: probing ? "pending" : "unknown",
                msg: "widget.unchecked",
              }
            : !p.widget
              ? {
                  status: "fail",
                  msg: p.feedback_widget ? "widget.feedback" : "widget.missing",
                }
              : p.widget_project && p.widget_project !== slug
                ? {
                    status: "fail",
                    msg: "widget.wrongProject",
                    values: { found: p.widget_project, expected: slug },
                  }
                : p.widget_app_origin !== origin
                  ? {
                      status: "fail",
                      msg: "widget.wrongOrigin",
                      values: { origin },
                    }
                  : { status: "ok", msg: "widget.ok" };
  const connected: Check = live
    ? { status: "ok", msg: "connected.ok" }
    : frame === "loading"
      ? { status: "pending", msg: "connected.pending" }
      : { status: "fail", msg: "connected.fail" };
  // Banner reason: the first failing check, framing first (nothing else matters while the frame is blocked).
  const reason = [framing, widget].find((c) => c.status === "fail");
  return { framing, widget, connected, reason };
}

const ICON = {
  ok: CircleCheck,
  fail: CircleAlert,
  unknown: CircleHelp,
  pending: Loader2,
} as const;
const TONE = {
  ok: "text-success-foreground",
  fail: "text-destructive",
  unknown: "text-muted-foreground",
  pending: "text-muted-foreground animate-spin",
} as const;

/** Setup checklist with live status: site URL → site allows framing → widget installed → connected. */
export function Setup({
  input,
  siteUrl,
  snippet,
  onRecheck,
  className,
}: {
  input: SetupInput;
  siteUrl: string;
  snippet: string;
  onRecheck: () => void;
  className?: string;
}) {
  const t = useTranslations("reviewPage.setup");
  const d = diagnose(input);
  const titleId = useId();
  const fix = `Content-Security-Policy: frame-ancestors 'self' ${input.origin}`;
  const steps: {
    id: "site" | "framing" | "widget" | "connected";
    check: Check;
    extra?: React.ReactNode;
  }[] = [
    {
      id: "site",
      check: { status: "ok", msg: "site.ok" },
      extra: (
        <p className="flex flex-wrap items-center gap-x-2 text-xs">
          <span dir="ltr" className="font-mono break-all">
            {siteUrl}
          </span>
          <Link
            href={`/app/${input.slug}/settings#preview`}
            className="font-medium underline underline-offset-4"
          >
            {t("site.change")}
          </Link>
        </p>
      ),
    },
    {
      id: "framing",
      check: d.framing,
      extra: d.framing.status === "fail" && (
        <>
          <p className="text-xs text-muted-foreground">{t("framing.fix")}</p>
          <CodeBlock code={fix} />
        </>
      ),
    },
    {
      id: "widget",
      check: d.widget,
      extra: (d.widget.status === "fail" || d.widget.status === "unknown") &&
        input.widgetError === null && (
          <>
            <p className="text-xs text-muted-foreground">{t("widget.paste")}</p>
            <CodeBlock code={snippet} />
          </>
        ),
    },
    { id: "connected", check: d.connected },
  ];

  return (
    <section className={cn("space-y-3", className)} aria-labelledby={titleId}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <h2 id={titleId} className="font-medium">
            {t("title")}
          </h2>
          <p className="text-sm text-pretty text-muted-foreground">
            {t("body")}
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          onClick={onRecheck}
          disabled={input.probing}
        >
          {input.probing ? <Loader2 className="animate-spin" /> : <RotateCw />}{" "}
          {t("recheck")}
        </Button>
      </div>
      <ol className="space-y-2">
        {steps.map(({ id, check, extra }, i) => {
          const Icon = ICON[check.status];
          return (
            <li key={id} className="flex gap-3 rounded-lg border p-3">
              <Icon
                aria-hidden
                className={cn("mt-0.5 size-4 shrink-0", TONE[check.status])}
              />
              <div className="min-w-0 flex-1 space-y-1.5">
                <p className="text-sm font-medium">
                  <span className="sr-only">
                    {t(`status.${check.status}`)}:{" "}
                  </span>
                  {i + 1}. {t(`${id}.title`)}
                </p>
                <p className="text-xs text-pretty text-muted-foreground">
                  {t(check.msg, check.values)}
                </p>
                {extra}
              </div>
            </li>
          );
        })}
      </ol>
      <details className="rounded-lg border p-3 text-sm">
        <summary className="cursor-pointer font-medium">
          {t("advanced.title")}
        </summary>
        <p className="mt-2 text-xs text-pretty text-muted-foreground">
          {t("advanced.body")}
        </p>
        <CodeBlock
          code={BRIDGE_SNIPPET}
          className="mt-2 max-h-32 overflow-y-auto"
        />
      </details>
    </section>
  );
}
