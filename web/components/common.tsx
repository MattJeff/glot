"use client";

import { Braces, Check, Copy, Languages, Monitor, Moon, RotateCw, Sun, TriangleAlert } from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { useTheme } from "next-themes";
import { useCallback, useState, useTransition } from "react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePathname, useRouter } from "@/i18n/navigation";
import { routing, type Locale } from "@/i18n/routing";
import type { TranslationStatus } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";

export function Logo({ className }: { className?: string }) {
  const t = useTranslations("common");
  return (
    <span className={cn("inline-flex items-center gap-2 font-semibold tracking-tight", className)}>
      <span aria-hidden className="grid size-6 place-items-center rounded-md bg-brand text-brand-foreground">
        <Braces className="size-3.5" strokeWidth={2.5} />
      </span>
      {t("brand")}
    </span>
  );
}

/** Language name in its own language (for switchers) or in the UI language (for data). */
export function useLocaleName() {
  const ui = useLocale();
  return useCallback(
    (code: string, native = false) => {
      try {
        return new Intl.DisplayNames([native ? code : ui], { type: "language" }).of(code) ?? code;
      } catch {
        return code;
      }
    },
    [ui],
  );
}

export function ThemeToggle() {
  const t = useTranslations("theme");
  const { theme, setTheme } = useTheme();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={t("label")}>
          <Sun className="dark:hidden" />
          <Moon className="hidden dark:block" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup value={theme} onValueChange={setTheme}>
          <DropdownMenuRadioItem value="light">
            <Sun /> {t("light")}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon /> {t("dark")}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="system">
            <Monitor /> {t("system")}
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function useSwitchLocale() {
  const router = useRouter();
  const pathname = usePathname();
  const [, start] = useTransition();
  return (locale: Locale) =>
    start(() => {
      // pathname from next-intl has no locale prefix; dynamic segments are already filled in.
      router.replace({ pathname, query: Object.fromEntries(new URLSearchParams(window.location.search)) }, { locale });
    });
}

export function LocaleSwitcher() {
  const t = useTranslations("localeSwitcher");
  const locale = useLocale();
  const name = useLocaleName();
  const switchLocale = useSwitchLocale();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={t("label")}>
          <Languages />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>{t("label")}</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={locale} onValueChange={(v) => switchLocale(v as Locale)}>
          {routing.locales.map((l) => (
            <DropdownMenuRadioItem key={l} value={l} lang={l}>
              <span className="capitalize">{name(l, true)}</span>
              <span className="ms-auto font-mono text-xs text-muted-foreground">{l}</span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function CopyButton({ value, className }: { value: string; className?: string }) {
  const t = useTranslations("common");
  const [done, setDone] = useState(false);
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      className={className}
      aria-label={done ? t("copied") : t("copy")}
      onClick={async () => {
        await navigator.clipboard.writeText(value);
        setDone(true);
        toast.success(t("copied"));
        setTimeout(() => setDone(false), 1500);
      }}
    >
      {done ? <Check key="done" className="text-success-foreground motion-safe:animate-pop" /> : <Copy key="copy" />}
    </Button>
  );
}

export function CodeBlock({ code, label, className }: { code: string; label?: string; className?: string }) {
  return (
    // Whole block is LTR so the copy button sits on the same side as the pre's end padding, even in RTL.
    <div dir="ltr" className={cn("group relative overflow-hidden rounded-lg border bg-muted/40", className)}>
      {label && (
        <div className="border-b px-3 py-1.5 text-xs text-muted-foreground">{label}</div>
      )}
      <pre tabIndex={0} className="overflow-x-auto rounded-b-lg p-3 pe-10 font-mono focus-visible:outline-2 focus-visible:-outline-offset-2 text-[13px] leading-relaxed">
        <code translate="no">{code}</code>
      </pre>
      <CopyButton value={code} className="absolute end-2 bottom-2" />
    </div>
  );
}

/** Page title + one sentence saying what the page is for (ux-brief §5). */
export function PageHeader({
  title,
  subtitle,
  actions,
  eyebrow,
}: {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
  eyebrow?: React.ReactNode;
}) {
  return (
    <div className="mb-8 flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
      <div className="min-w-0 max-w-2xl">
        {eyebrow && <div className="mb-1 text-xs font-medium text-muted-foreground">{eyebrow}</div>}
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        {subtitle && <p className="mt-1.5 text-sm text-pretty text-muted-foreground">{subtitle}</p>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

/**
 * Teaching empty state: what's missing, why it matters, the one action, optionally a snippet/doc link.
 * The icon sits on a small decorative "orbit" illustration (pure CSS, aria-hidden).
 */
export function EmptyState({
  icon: Icon,
  title,
  body,
  action,
  children,
  className,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  body?: React.ReactNode;
  action?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "relative flex flex-col items-center justify-center overflow-hidden rounded-xl border border-dashed px-6 py-14 text-center motion-safe:animate-rise",
        className,
      )}
    >
      <div aria-hidden className="relative mb-5 grid size-20 place-items-center">
        <span className="absolute inset-0 rounded-full border border-dashed border-border" />
        <span className="absolute inset-3 rounded-full bg-[radial-gradient(circle,color-mix(in_oklch,var(--brand)_18%,transparent),transparent_70%)]" />
        <span className="absolute top-1 end-2 size-1.5 rounded-full bg-brand/70" />
        <span className="absolute bottom-2 start-1 size-1 rounded-full bg-info/70" />
        <span className="relative grid size-11 place-items-center rounded-xl border bg-card shadow-(--shadow-card)">
          <Icon className="size-5 text-muted-foreground" />
        </span>
      </div>
      <h2 className="text-base font-medium">{title}</h2>
      {body && <p className="mt-1.5 max-w-md text-sm text-pretty text-muted-foreground">{body}</p>}
      {action && <div className="mt-5 flex flex-wrap justify-center gap-2">{action}</div>}
      {children && <div className="mt-6 w-full max-w-lg text-start">{children}</div>}
    </div>
  );
}

/** A glossary-style term: dotted underline, focusable, explains itself in a tooltip (lockfile, pixel budget…). */
export function TermTip({ children, tip }: { children: React.ReactNode; tip: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="cursor-help rounded-sm underline decoration-muted-foreground/60 decoration-dotted underline-offset-4"
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-72 text-pretty">{tip}</TooltipContent>
    </Tooltip>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const t = useTranslations("common");
  const message = useErrorMessage();
  return (
    <div role="alert" className="flex flex-col items-center gap-3 rounded-xl border border-destructive/30 bg-destructive/5 px-6 py-10 text-center">
      <TriangleAlert className="size-5 text-destructive" aria-hidden />
      <p className="max-w-md text-sm">{message(error)}</p>
      {onRetry && (
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RotateCw /> {t("retry")}
        </Button>
      )}
    </div>
  );
}

const STATUS_STYLE: Record<TranslationStatus, string> = {
  missing: "bg-destructive/10 text-destructive",
  machine: "bg-info/12 text-info-foreground",
  reviewed: "bg-success/12 text-success-foreground",
  stale: "bg-warning/15 text-warning-foreground",
};
const STATUS_DOT: Record<TranslationStatus, string> = {
  missing: "bg-destructive",
  machine: "bg-info",
  reviewed: "bg-success",
  stale: "bg-warning",
};

export function StatusBadge({ status, className }: { status: TranslationStatus; className?: string }) {
  const t = useTranslations("status");
  return (
    <Badge variant="secondary" className={cn("h-5 gap-1.5 border-0 px-1.5 text-[11px] font-medium", STATUS_STYLE[status], className)}>
      <span aria-hidden className={cn("size-1.5 rounded-full", STATUS_DOT[status])} />
      {t(status)}
    </Badge>
  );
}

/** Stacked bar: reviewed (solid) + machine-translated (lighter) + stale (amber) over total. Segments grow in (transform only). */
export function ProgressBar({
  total,
  reviewed,
  translated,
  stale = 0,
  className,
  label,
}: {
  total: number;
  reviewed: number;
  translated: number;
  stale?: number;
  className?: string;
  label?: string;
}) {
  const pct = (n: number) => (total > 0 ? Math.min(100, (n / total) * 100) : 0);
  const machine = Math.max(0, translated - reviewed - stale);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={translated}
      className={cn("h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
    >
      <div className="flex h-full origin-left motion-safe:animate-[grow_700ms_var(--ease-out-quint)_both] rtl:origin-right">
        <div className="bg-success" style={{ width: `${pct(reviewed)}%` }} />
        <div className="bg-success/45" style={{ width: `${pct(machine)}%` }} />
        <div className="bg-warning" style={{ width: `${pct(stale)}%` }} />
      </div>
    </div>
  );
}
