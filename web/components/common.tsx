"use client";

import { Braces, Check, Copy, Languages, Monitor, Moon, Sun, TriangleAlert } from "lucide-react";
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
      aria-label={t("copy")}
      onClick={async () => {
        await navigator.clipboard.writeText(value);
        setDone(true);
        toast.success(t("copied"));
        setTimeout(() => setDone(false), 1500);
      }}
    >
      {done ? <Check /> : <Copy />}
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
      <pre className="overflow-x-auto p-3 pe-10 font-mono text-[13px] leading-relaxed">
        <code>{code}</code>
      </pre>
      <CopyButton value={code} className="absolute end-2 bottom-2" />
    </div>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
      <div className="min-w-0">
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {subtitle && <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  body,
  action,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  body?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed px-6 py-14 text-center">
      <div className="mb-3 grid size-10 place-items-center rounded-lg border bg-muted/50">
        <Icon className="size-5 text-muted-foreground" />
      </div>
      <h3 className="font-medium">{title}</h3>
      {body && <p className="mt-1 max-w-sm text-sm text-muted-foreground">{body}</p>}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const t = useTranslations("common");
  const message = useErrorMessage();
  return (
    <div role="alert" className="flex flex-col items-center gap-3 rounded-xl border border-destructive/30 bg-destructive/5 px-6 py-10 text-center">
      <TriangleAlert className="size-5 text-destructive" />
      <p className="text-sm">{message(error)}</p>
      {onRetry && (
        <Button variant="outline" size="sm" onClick={onRetry}>
          {t("retry")}
        </Button>
      )}
    </div>
  );
}

const STATUS_STYLE: Record<TranslationStatus, string> = {
  missing: "bg-destructive/10 text-destructive",
  machine: "bg-info/12 text-info",
  reviewed: "bg-success/12 text-success",
  stale: "bg-warning/15 text-warning",
};

export function StatusBadge({ status, className }: { status: TranslationStatus; className?: string }) {
  const t = useTranslations("status");
  return (
    <Badge variant="secondary" className={cn("h-5 border-0 px-1.5 text-[11px] font-medium", STATUS_STYLE[status], className)}>
      {t(status)}
    </Badge>
  );
}

/** Stacked bar: reviewed (solid) + machine-translated (lighter) + stale (amber) over total. */
export function ProgressBar({
  total,
  reviewed,
  translated,
  stale = 0,
  className,
}: {
  total: number;
  reviewed: number;
  translated: number;
  stale?: number;
  className?: string;
}) {
  const pct = (n: number) => (total > 0 ? Math.min(100, (n / total) * 100) : 0);
  const machine = Math.max(0, translated - reviewed - stale);
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={translated}
      className={cn("flex h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
    >
      <div className="bg-success" style={{ width: `${pct(reviewed)}%` }} />
      <div className="bg-success/45" style={{ width: `${pct(machine)}%` }} />
      <div className="bg-warning" style={{ width: `${pct(stale)}%` }} />
    </div>
  );
}
