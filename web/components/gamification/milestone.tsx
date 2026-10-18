"use client";

import { Check } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { Link } from "@/i18n/navigation";
import { cn } from "@/lib/utils";

/**
 * A real, earned milestone (docs/design/ux-brief.md §2): never arbitrary points.
 * Render a few inside <MilestoneList>. `progress` shows how far along an unachieved one is;
 * `href` makes it actionable ("how to earn it").
 *
 *   <MilestoneList>
 *     <Milestone icon={Upload} title="First push" description="Send your source strings" achieved />
 *     <Milestone icon={Languages} title="First locale at 100 %" progress={{ value: 87, max: 125 }} href="/app/acme/keys" />
 *   </MilestoneList>
 */
export function Milestone({
  icon: Icon,
  title,
  description,
  achieved = false,
  progress,
  href,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  description?: string;
  achieved?: boolean;
  progress?: { value: number; max: number };
  href?: string;
}) {
  const t = useTranslations("gamification");
  const format = useFormatter();
  const pct = progress && progress.max > 0 ? Math.min(100, (progress.value / progress.max) * 100) : achieved ? 100 : 0;
  const body = (
    <>
      <span
        aria-hidden
        className={cn(
          "relative grid size-9 shrink-0 place-items-center rounded-full border transition-colors",
          achieved ? "border-transparent bg-brand text-brand-foreground motion-safe:animate-pop" : "bg-muted/60 text-muted-foreground",
        )}
      >
        <Icon className="size-4" />
        {achieved && (
          <span className="absolute -end-0.5 -bottom-0.5 grid size-4 place-items-center rounded-full bg-background ring-2 ring-background">
            <Check className="size-3 text-success-foreground" strokeWidth={3} />
          </span>
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className={cn("block text-sm font-medium", !achieved && "text-foreground/90")}>{title}</span>
        {description && <span className="mt-0.5 block text-xs text-muted-foreground">{description}</span>}
        {!achieved && progress && (
          <span className="mt-2 flex items-center gap-2">
            <span className="h-1 flex-1 overflow-hidden rounded-full bg-muted">
              <span
                className="block h-full origin-left rounded-full bg-brand transition-transform duration-(--duration-data) ease-(--ease-out-quint) rtl:origin-right"
                style={{ transform: `scaleX(${pct / 100})` }}
              />
            </span>
            <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
              {format.number(progress.value)}/{format.number(progress.max)}
            </span>
          </span>
        )}
      </span>
      <span className="sr-only">{achieved ? t("achieved") : t("notYet")}</span>
    </>
  );
  const cls = "flex items-start gap-3 rounded-lg px-3 py-2.5";
  return (
    <li>
      {href && !achieved ? (
        <Link href={href} className={cn(cls, "transition-colors hover:bg-accent/60")}>
          {body}
        </Link>
      ) : (
        <div className={cls}>{body}</div>
      )}
    </li>
  );
}

export function MilestoneList({ label, children, className }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <ul aria-label={label} className={cn("grid gap-1 sm:grid-cols-2", className)}>
      {children}
    </ul>
  );
}
