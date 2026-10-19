"use client";

import {
  CheckCheck,
  Circle,
  CircleCheck,
  CircleDot,
  CircleSlash,
  MessageSquare,
  Search,
  SlidersHorizontal,
} from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useMemo } from "react";
import { useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
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
  ANNOTATION_STATUSES,
  type Annotation,
  type AnnotationStatus,
} from "@/lib/api";
import { cn } from "@/lib/utils";

export const ALL = "all";
/** Status filter: the two working sets first, then each status. */
export const STATUS_FILTERS = [
  "active",
  "done",
  ALL,
  ...ANNOTATION_STATUSES,
] as const;
export type StatusFilter = (typeof STATUS_FILTERS)[number];
export type Filters = {
  status: StatusFilter;
  route: string;
  locale: string;
  author: string;
  q: string;
};
export const DEFAULT_FILTERS: Filters = {
  status: "active",
  route: ALL,
  locale: ALL,
  author: ALL,
  q: "",
};

export const isDone = (s: AnnotationStatus) =>
  s === "resolved" || s === "wontfix";
const STATUS_ICON = {
  open: Circle,
  in_progress: CircleDot,
  resolved: CircleCheck,
  wontfix: CircleSlash,
} as const;
export const STATUS_STYLE: Record<AnnotationStatus, string> = {
  open: "bg-info/12 text-info-foreground",
  in_progress: "bg-warning/15 text-warning-foreground",
  resolved: "bg-success/12 text-success-foreground",
  wontfix: "bg-muted text-muted-foreground",
};

/** "5 minutes ago", ticking every minute. */
export function Ago({ date }: { date: string }) {
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  return (
    <time dateTime={date}>{format.relativeTime(new Date(date), now)}</time>
  );
}

export function StatusPill({
  status,
  className,
}: {
  status: AnnotationStatus;
  className?: string;
}) {
  const t = useTranslations("reviewPage.status");
  const Icon = STATUS_ICON[status];
  return (
    <Badge
      variant="secondary"
      className={cn(
        "h-5 gap-1 border-0 px-1.5 text-[11px] font-medium",
        STATUS_STYLE[status],
        className,
      )}
    >
      <Icon aria-hidden className="size-3" />
      {t(status)}
    </Badge>
  );
}

export function matches(a: Annotation, f: Filters) {
  if (f.status === "active" && isDone(a.status)) return false;
  if (f.status === "done" && !isDone(a.status)) return false;
  if (
    f.status !== "active" &&
    f.status !== "done" &&
    f.status !== ALL &&
    a.status !== f.status
  )
    return false;
  if (f.route !== ALL && a.route !== f.route) return false;
  if (f.locale !== ALL && a.locale !== f.locale) return false;
  if (f.author !== ALL && (a.author.user_id ?? a.author.name) !== f.author)
    return false;
  const q = f.q.trim().toLowerCase();
  if (
    q &&
    ![
      a.body,
      a.anchor.text,
      a.key ?? "",
      a.suggestion ?? "",
      a.author.name,
    ].some((s) => s.toLowerCase().includes(q))
  )
    return false;
  return true;
}

/** Filters + search + the list itself. Items are buttons; J/K move the selection (handled by the workspace). */
export function AnnotationList({
  items,
  all,
  numbers,
  filters,
  onFilters,
  selected,
  onSelect,
  resolvedThisWeek,
  emptySetup,
  emptyAction,
}: {
  items: Annotation[];
  all: Annotation[];
  numbers: Map<string, number>;
  filters: Filters;
  onFilters: (f: Filters) => void;
  selected: string | null;
  onSelect: (id: string) => void;
  resolvedThisWeek: number;
  /** Setup checklist, shown while the site isn't connected yet. */
  emptySetup: React.ReactNode;
  emptyAction: React.ReactNode;
}) {
  const t = useTranslations("reviewPage.list");
  const ts = useTranslations("reviewPage.status");
  const localeName = useLocaleName();
  const routes = useMemo(
    () => [...new Set(all.map((a) => a.route))].sort(),
    [all],
  );
  const locales = useMemo(
    () => [...new Set(all.map((a) => a.locale))].sort(),
    [all],
  );
  const authors = useMemo(() => {
    const m = new Map<string, string>();
    for (const a of all)
      m.set(a.author.user_id ?? a.author.name, a.author.name);
    return [...m];
  }, [all]);
  // Counts per option, under the other active filters.
  const countFor = (patch: Partial<Filters>) =>
    all.filter((a) => matches(a, { ...filters, ...patch })).length;
  const set = (patch: Partial<Filters>) => onFilters({ ...filters, ...patch });
  const filtered =
    JSON.stringify({ ...filters, q: "" }) !==
      JSON.stringify({ ...DEFAULT_FILTERS, q: "" }) || !!filters.q;

  if (all.length === 0)
    return (
      <div className="flex flex-col gap-4 p-4">
        <div className="rounded-xl border border-dashed p-5 text-center">
          <MessageSquare
            aria-hidden
            className="mx-auto mb-3 size-8 text-muted-foreground"
          />
          <h2 className="font-medium">{t("empty.title")}</h2>
          <p className="mt-1 text-sm text-pretty text-muted-foreground">
            {t("empty.body")}
          </p>
          <div className="mt-4 flex justify-center">{emptyAction}</div>
        </div>
        {emptySetup}
      </div>
    );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="space-y-2 border-b p-3">
        {resolvedThisWeek > 0 && (
          <p className="flex items-center gap-1.5 text-xs text-success-foreground">
            <CheckCheck aria-hidden className="size-3.5" />
            {t("resolvedThisWeek", { count: resolvedThisWeek })}
          </p>
        )}
        <div className="relative">
          <Label htmlFor="rv-search" className="sr-only">
            {t("search")}
          </Label>
          <Search
            aria-hidden
            className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            id="rv-search"
            type="search"
            value={filters.q}
            onChange={(e) => set({ q: e.target.value })}
            placeholder={t("searchPlaceholder")}
            className="h-8 ps-8"
          />
        </div>
        <div
          role="group"
          aria-label={t("filters")}
          className="flex flex-wrap items-center gap-1.5"
        >
          <SlidersHorizontal
            aria-hidden
            className="size-3.5 text-muted-foreground"
          />
          <FilterSelect
            label={t("status")}
            value={filters.status}
            onChange={(v) => set({ status: v as StatusFilter })}
          >
            {STATUS_FILTERS.map((s) => (
              <SelectItem key={s} value={s}>
                {s === "active" || s === "done" || s === ALL
                  ? t(`statusFilter.${s}`)
                  : ts(s)}{" "}
                <span className="text-muted-foreground tabular-nums">
                  {countFor({ status: s })}
                </span>
              </SelectItem>
            ))}
          </FilterSelect>
          <FilterSelect
            label={t("route")}
            value={filters.route}
            onChange={(v) => set({ route: v })}
          >
            <SelectItem value={ALL}>{t("allRoutes")}</SelectItem>
            {routes.map((r) => (
              <SelectItem key={r} value={r}>
                <span dir="ltr" className="font-mono">
                  {r}
                </span>{" "}
                <span className="text-muted-foreground tabular-nums">
                  {countFor({ route: r })}
                </span>
              </SelectItem>
            ))}
          </FilterSelect>
          <FilterSelect
            label={t("locale")}
            value={filters.locale}
            onChange={(v) => set({ locale: v })}
          >
            <SelectItem value={ALL}>{t("allLocales")}</SelectItem>
            {locales.map((l) => (
              <SelectItem key={l} value={l}>
                {localeName(l)}{" "}
                <span className="font-mono text-xs text-muted-foreground">
                  {l}
                </span>{" "}
                <span className="text-muted-foreground tabular-nums">
                  {countFor({ locale: l })}
                </span>
              </SelectItem>
            ))}
          </FilterSelect>
          <FilterSelect
            label={t("author")}
            value={filters.author}
            onChange={(v) => set({ author: v })}
          >
            <SelectItem value={ALL}>{t("allAuthors")}</SelectItem>
            {authors.map(([id, name]) => (
              <SelectItem key={id} value={id}>
                {name}
              </SelectItem>
            ))}
          </FilterSelect>
        </div>
      </div>
      <div className="flex items-center gap-2 border-b px-3 py-1.5 text-xs text-muted-foreground">
        <p role="status" className="tabular-nums">
          {t("count", { count: items.length })}
          {items.length !== all.length && ` / ${all.length}`}
        </p>
        {filtered && (
          <button
            type="button"
            className="ms-auto rounded-sm font-medium text-foreground underline underline-offset-4"
            onClick={() => onFilters(DEFAULT_FILTERS)}
          >
            {t("clearFilters")}
          </button>
        )}
      </div>
      {items.length === 0 ? (
        <div className="p-6 text-center text-sm text-muted-foreground">
          <p>{t("noMatch")}</p>
          {filtered && (
            <button
              type="button"
              className="mt-2 rounded-sm font-medium text-foreground underline underline-offset-4"
              onClick={() => onFilters(DEFAULT_FILTERS)}
            >
              {t("clearFilters")}
            </button>
          )}
        </div>
      ) : (
        <ul
          className="min-h-0 flex-1 divide-y overflow-y-auto"
          aria-label={t("label")}
        >
          {items.map((a) => (
            <li key={a.id}>
              <button
                type="button"
                data-annotation={a.id}
                aria-current={a.id === selected ? "true" : undefined}
                onClick={() => onSelect(a.id)}
                className={cn(
                  "flex w-full gap-3 px-3 py-3 text-start transition-colors duration-150 hover:bg-accent focus-visible:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset",
                  a.id === selected && "bg-accent",
                )}
              >
                <span
                  aria-hidden
                  className={cn(
                    "mt-0.5 grid size-6 shrink-0 place-items-center rounded-full text-[11px] font-semibold tabular-nums",
                    isDone(a.status)
                      ? "bg-muted text-muted-foreground"
                      : "bg-brand text-brand-foreground",
                  )}
                >
                  {numbers.get(a.id)}
                </span>
                <span className="min-w-0 flex-1 space-y-1">
                  <span className="sr-only">
                    {t("number", { n: numbers.get(a.id) ?? 0 })}
                  </span>
                  <span className="line-clamp-2 text-sm">{a.body}</span>
                  {a.anchor.text && (
                    <span className="block truncate text-xs text-muted-foreground">
                      “{a.anchor.text}”
                    </span>
                  )}
                  <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                    <StatusPill status={a.status} />
                    <span dir="ltr" className="font-mono">
                      {a.route}
                    </span>
                    <span>{a.locale}</span>
                    {a.key && (
                      <span dir="ltr" className="max-w-40 truncate font-mono">
                        {a.key}
                      </span>
                    )}
                  </span>
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span className="truncate">{a.author.name}</span>
                    <Ago date={a.updated_at} />
                    {a.comment_count > 0 && (
                      <span className="ms-auto flex items-center gap-1 tabular-nums">
                        <MessageSquare aria-hidden className="size-3" />
                        <span className="sr-only">
                          {t("replies", { count: a.comment_count })}
                        </span>
                        <span aria-hidden>{a.comment_count}</span>
                      </span>
                    )}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function FilterSelect({
  label,
  value,
  onChange,
  children,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  children: React.ReactNode;
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger
        size="sm"
        aria-label={label}
        className="h-7 max-w-36 text-xs"
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>{children}</SelectContent>
    </Select>
  );
}
