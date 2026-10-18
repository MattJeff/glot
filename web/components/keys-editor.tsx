"use client";

import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { createColumnHelper, flexRender, getCoreRowModel, useReactTable } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Check,
  CircleAlert,
  CircleX,
  Columns3,
  FileCode,
  FilterX,
  PanelRight,
  Search,
  SearchX,
} from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, StatusBadge, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  api,
  type Issue,
  type KeyPage,
  type KeyRow,
  type Project,
  qk,
  STATUSES,
  type Translation,
  type TranslationStatus,
} from "@/lib/api";
import { cn } from "@/lib/utils";

const PER_PAGE = 100;
const MISSING: Translation = {
  value: "",
  status: "missing",
  score: null,
  issues: [],
  origin: "machine",
  model: null,
  updated_at: "",
};
const tr = (row: KeyRow, locale: string) => row.translations[locale] ?? MISSING;

function patchTranslation(qc: QueryClient, slug: string, key: string, locale: string, next: Translation) {
  qc.setQueriesData<InfiniteData<KeyPage>>({ queryKey: qk.keys(slug) }, (d) =>
    d && {
      ...d,
      pages: d.pages.map((p) => ({
        ...p,
        items: p.items.map((r) => (r.key === key ? { ...r, translations: { ...r.translations, [locale]: next } } : r)),
      })),
    },
  );
  qc.invalidateQueries({ queryKey: qk.project(slug) });
  qc.invalidateQueries({ queryKey: qk.issues(slug) });
  qc.invalidateQueries({ queryKey: qk.history(slug, key) });
}

const ORIGINS = ["machine", "human", "imported"] as const;
const isOrigin = (o: string): o is (typeof ORIGINS)[number] => (ORIGINS as readonly string[]).includes(o);

const isTyping = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));

export function KeysEditor({ project }: { project: Project }) {
  const t = useTranslations("editor");
  const ts = useTranslations("status");
  const format = useFormatter();
  const localeName = useLocaleName();
  const qc = useQueryClient();
  const slug = project.slug;
  const params = useSearchParams();

  // Filters (initialised from the URL so overview links deep-link here).
  const [q, setQ] = useState(params.get("q") ?? "");
  const [debouncedQ, setDebouncedQ] = useState(q);
  const [locale, setLocale] = useState(params.get("locale") ?? "all");
  const [status, setStatus] = useState<TranslationStatus | "all">(
    (STATUSES as string[]).includes(params.get("status") ?? "") ? (params.get("status") as TranslationStatus) : "all",
  );
  const [withIssues, setWithIssues] = useState(params.get("issues") === "1");
  const [visible, setVisible] = useState<string[]>(() =>
    locale !== "all" ? [locale] : project.target_locales.slice(0, 3),
  );

  useEffect(() => {
    const id = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(id);
  }, [q]);

  useEffect(() => {
    const s = new URLSearchParams();
    if (debouncedQ) s.set("q", debouncedQ);
    if (locale !== "all") s.set("locale", locale);
    if (status !== "all") s.set("status", status);
    if (withIssues) s.set("issues", "1");
    const qs = s.toString();
    window.history.replaceState(null, "", qs ? `?${qs}` : window.location.pathname);
  }, [debouncedQ, locale, status, withIssues]);

  const filters = { q: debouncedQ, locale: locale === "all" ? undefined : locale, status: status === "all" ? undefined : status };
  const keys = useInfiniteQuery({
    queryKey: [...qk.keys(slug), filters],
    queryFn: ({ pageParam }) => api.keys(slug, { ...filters, page: pageParam, per_page: PER_PAGE }),
    initialPageParam: 1,
    getNextPageParam: (last, pages) => (pages.length * PER_PAGE < last.total ? pages.length + 1 : undefined),
  });
  const total = keys.data?.pages[0]?.total ?? 0;

  const rows = useMemo(() => {
    const all = keys.data?.pages.flatMap((p) => p.items) ?? [];
    // ponytail: "with issues" filters loaded pages client-side; the API has no issues filter yet.
    return withIssues ? all.filter((r) => visible.some((l) => tr(r, l).issues.length > 0)) : all;
  }, [keys.data, withIssues, visible]);

  // Selection / editing
  const [active, setActive] = useState({ row: 0, col: 0 });
  const [editing, setEditing] = useState<{ key: string; locale: string } | null>(null);
  const [draft, setDraft] = useState("");
  // Rendered client-only (after the project query resolves), so reading matchMedia here is hydration-safe.
  const [detailOpen, setDetailOpen] = useState(() => window.matchMedia("(min-width: 1280px)").matches);
  const activeRow = rows[Math.min(active.row, rows.length - 1)];
  const activeLocale = visible[Math.min(active.col, visible.length - 1)];

  const save = useMutation({
    mutationFn: (v: { key: string; locale: string; value: string }) => api.putTranslation(slug, v.key, v.locale, v.value),
    onSuccess: (next, v) => {
      patchTranslation(qc, slug, v.key, v.locale, next);
      setEditing(null);
      if (next.issues.length) toast.warning(t("savedWithIssues", { count: next.issues.length }));
      else toast.success(t("saved"));
    },
  });
  const review = useMutation({
    mutationFn: (v: { key: string; locale: string }) => api.review(slug, v.key, v.locale),
    onSuccess: (next, v) => {
      patchTranslation(qc, slug, v.key, v.locale, next);
      toast.success(t("approved"));
    },
  });

  const startEdit = useCallback((row: KeyRow, loc: string) => {
    setEditing({ key: row.key, locale: loc });
    setDraft(tr(row, loc).value);
  }, []);

  // Table model
  const columns = useMemo(() => {
    const h = createColumnHelper<KeyRow>();
    return [
      h.accessor("key", {
        header: () => t("key"),
        cell: (c) => (
          <div className="min-w-0">
            <div className="truncate font-mono text-xs" title={c.getValue()}>{c.getValue()}</div>
            {c.row.original.description && (
              <div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{c.row.original.description}</div>
            )}
          </div>
        ),
      }),
      h.accessor("source", {
        header: () => (
          <>
            {t("source")} <span className="font-mono text-muted-foreground">{project.source_locale}</span>
          </>
        ),
        cell: (c) => (
          <p dir="auto" lang={project.source_locale} className="line-clamp-3 text-sm whitespace-pre-wrap">
            {c.getValue()}
          </p>
        ),
      }),
      ...project.target_locales.map((l) =>
        h.display({
          id: l,
          header: () => (
            <>
              <span className="font-mono">{l}</span> <span className="text-muted-foreground capitalize">{localeName(l)}</span>
            </>
          ),
        }),
      ),
    ];
  }, [project.source_locale, project.target_locales, t, localeName]);

  const table = useReactTable({
    data: rows,
    columns,
    getCoreRowModel: getCoreRowModel(),
    getRowId: (r) => r.key,
    state: { columnVisibility: Object.fromEntries(project.target_locales.map((l) => [l, visible.includes(l)])) },
  });
  const tableRows = table.getRowModel().rows;
  const gridTemplate = `minmax(12rem,16rem) minmax(14rem,1fr) ${visible.map(() => "minmax(15rem,1fr)").join(" ")}`;

  // Virtualisation + infinite loading
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: tableRows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 72,
    overscan: 8,
  });
  const items = virtualizer.getVirtualItems();
  const lastIndex = items.at(-1)?.index ?? 0;
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = keys;
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && lastIndex >= tableRows.length - 15) fetchNextPage();
  }, [lastIndex, tableRows.length, hasNextPage, isFetchingNextPage, fetchNextPage]);

  // Keyboard: j/k rows, h/l locales, Enter edit, ⌘Enter approve, i details, / search
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || document.querySelector("[role=dialog]")) return;
      const move = (dr: number, dc: number) => {
        e.preventDefault();
        setActive((a) => {
          const row = Math.max(0, Math.min(rows.length - 1, a.row + dr));
          virtualizer.scrollToIndex(row, { align: "auto" });
          return { row, col: Math.max(0, Math.min(visible.length - 1, a.col + dc)) };
        });
      };
      if (e.key === "j" || e.key === "ArrowDown") move(1, 0);
      else if (e.key === "k" || e.key === "ArrowUp") move(-1, 0);
      else if (e.key === "l" || e.key === "ArrowRight") move(0, document.dir === "rtl" ? -1 : 1);
      else if (e.key === "h" || e.key === "ArrowLeft") move(0, document.dir === "rtl" ? 1 : -1);
      else if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && activeRow && activeLocale) {
        e.preventDefault();
        review.mutate({ key: activeRow.key, locale: activeLocale });
      } else if (e.key === "Enter" && activeRow && activeLocale) {
        e.preventDefault();
        startEdit(activeRow, activeLocale);
      } else if (e.key === "i") setDetailOpen((v) => !v);
      else if (e.key === "/") {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rows.length, visible.length, activeRow, activeLocale, review, startEdit, virtualizer]);

  const hasFilters = !!q || locale !== "all" || status !== "all" || withIssues;
  const clearFilters = () => {
    setQ("");
    setLocale("all");
    setStatus("all");
    setWithIssues(false);
  };

  return (
    <div className="-mx-4 -my-6 flex h-[calc(100dvh-3rem)] flex-col sm:-mx-6 lg:-mx-8">
      {/* Toolbar */}
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2.5 sm:px-6">
        <h1 className="me-2 text-base font-semibold">{t("title")}</h1>
        <div className="relative w-full sm:w-64">
          <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={searchRef}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && e.currentTarget.blur()}
            placeholder={t("search")}
            aria-label={t("search")}
            className="h-8 ps-8"
          />
        </div>
        <Select
          value={locale}
          onValueChange={(v) => {
            setLocale(v);
            if (v !== "all" && !visible.includes(v)) setVisible((cur) => [v, ...cur]);
          }}
        >
          <SelectTrigger size="sm" className="w-36" aria-label={t("anyLocale")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("anyLocale")}</SelectItem>
            {project.target_locales.map((l) => (
              <SelectItem key={l} value={l}>
                <span className="font-mono text-xs">{l}</span> <span className="capitalize">{localeName(l)}</span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={status} onValueChange={(v) => setStatus(v as TranslationStatus | "all")}>
          <SelectTrigger size="sm" className="w-32" aria-label={t("anyStatus")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("anyStatus")}</SelectItem>
            {STATUSES.map((s) => (
              <SelectItem key={s} value={s}>
                {ts(s)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex items-center gap-2 px-1">
          <Switch id="with-issues" checked={withIssues} onCheckedChange={setWithIssues} size="sm" />
          <Label htmlFor="with-issues" className="text-sm font-normal">
            {t("withIssues")}
          </Label>
        </div>
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="outline" size="sm">
              <Columns3 /> {t("columns")}
              <span className="font-mono text-xs text-muted-foreground">{visible.length}</span>
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-56 p-2">
            <p className="px-2 pb-2 text-xs text-muted-foreground">{t("columnsHint")}</p>
            {project.target_locales.map((l) => (
              <label key={l} className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-accent">
                <Checkbox
                  checked={visible.includes(l)}
                  onCheckedChange={(c) =>
                    setVisible((cur) =>
                      c ? project.target_locales.filter((x) => x === l || cur.includes(x)) : cur.filter((x) => x !== l),
                    )
                  }
                />
                <span className="font-mono text-xs">{l}</span>
                <span className="text-muted-foreground capitalize">{localeName(l)}</span>
              </label>
            ))}
          </PopoverContent>
        </Popover>
        <div className="ms-auto flex items-center gap-2">
          <span className="text-xs text-muted-foreground tabular-nums">{t("results", { count: total })}</span>
          <Button
            variant={detailOpen ? "secondary" : "ghost"}
            size="icon-sm"
            aria-label={t("detail.toggle")}
            aria-pressed={detailOpen}
            onClick={() => setDetailOpen((v) => !v)}
          >
            <PanelRight className="rtl:-scale-x-100" />
          </Button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        {/* Grid */}
        <div ref={scrollRef} className="min-w-0 flex-1 overflow-auto" role="grid" aria-rowcount={total} aria-label={t("title")}>
          <div style={{ minWidth: "fit-content" }}>
            <div
              role="row"
              className="sticky top-0 z-10 grid border-b bg-background/95 text-xs font-medium text-muted-foreground backdrop-blur"
              style={{ gridTemplateColumns: gridTemplate }}
            >
              {table.getHeaderGroups()[0].headers.map((h) => (
                <div key={h.id} role="columnheader" className="truncate px-3 py-2">
                  {flexRender(h.column.columnDef.header, h.getContext())}
                </div>
              ))}
            </div>

            {keys.isPending ? (
              <div className="space-y-px">
                {Array.from({ length: 10 }, (_, i) => (
                  <div key={i} className="grid gap-3 border-b px-3 py-3" style={{ gridTemplateColumns: gridTemplate }}>
                    {Array.from({ length: 2 + visible.length }, (_, j) => (
                      <Skeleton key={j} className="h-9" />
                    ))}
                  </div>
                ))}
              </div>
            ) : keys.isError ? (
              <div className="p-6">
                <ErrorState error={keys.error} onRetry={() => keys.refetch()} />
              </div>
            ) : tableRows.length === 0 ? (
              <div className="p-6">
                <EmptyState
                  icon={SearchX}
                  title={t("empty")}
                  action={
                    hasFilters && (
                      <Button variant="outline" size="sm" onClick={clearFilters}>
                        <FilterX /> {t("clearFilters")}
                      </Button>
                    )
                  }
                />
              </div>
            ) : (
              <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
                {items.map((vi) => {
                  const row = tableRows[vi.index];
                  const isActiveRow = vi.index === active.row;
                  return (
                    <div
                      key={row.id}
                      ref={virtualizer.measureElement}
                      data-index={vi.index}
                      role="row"
                      aria-rowindex={vi.index + 2}
                      aria-selected={isActiveRow}
                      className={cn(
                        "absolute inset-x-0 grid border-b text-sm",
                        isActiveRow ? "bg-accent/50" : "hover:bg-accent/25",
                      )}
                      style={{ transform: `translateY(${vi.start}px)`, gridTemplateColumns: gridTemplate }}
                      onClick={() => setActive((a) => ({ ...a, row: vi.index }))}
                    >
                      {row.getVisibleCells().map((cell) => {
                        const loc = cell.column.id;
                        const col = visible.indexOf(loc);
                        if (col === -1) {
                          return (
                            <div key={cell.id} role="gridcell" className="min-w-0 px-3 py-2.5">
                              {flexRender(cell.column.columnDef.cell, cell.getContext())}
                            </div>
                          );
                        }
                        const isEditing = editing?.key === row.original.key && editing.locale === loc;
                        return (
                          <TranslationCell
                            key={cell.id}
                            locale={loc}
                            translation={tr(row.original, loc)}
                            active={isActiveRow && col === active.col}
                            editing={isEditing}
                            draft={draft}
                            saving={save.isPending}
                            onDraft={setDraft}
                            onFocus={() => setActive({ row: vi.index, col })}
                            onEdit={() => {
                              setActive({ row: vi.index, col });
                              startEdit(row.original, loc);
                            }}
                            onCancel={() => setEditing(null)}
                            onSave={() => save.mutate({ key: row.original.key, locale: loc, value: draft })}
                            onApprove={() => review.mutate({ key: row.original.key, locale: loc })}
                          />
                        );
                      })}
                    </div>
                  );
                })}
              </div>
            )}
            {keys.isFetchingNextPage && (
              <p className="py-3 text-center text-xs text-muted-foreground">{t("loadMore")}</p>
            )}
          </div>
        </div>

        {detailOpen && (
          <aside className="fixed inset-y-0 end-0 z-40 w-full max-w-sm overflow-y-auto border-s bg-background shadow-xl xl:static xl:z-auto xl:w-96 xl:max-w-none xl:shadow-none">
            <DetailPanel slug={slug} project={project} row={activeRow} onClose={() => setDetailOpen(false)} />
          </aside>
        )}
      </div>

      <footer className="hidden border-t px-6 py-1.5 text-[11px] text-muted-foreground md:block">
        {t("shortcuts")}
        <span dir="ltr" className="float-end font-mono">{format.number(rows.length)} / {format.number(total)}</span>
      </footer>
    </div>
  );
}

function IssuesIcon({ issues }: { issues: Issue[] }) {
  const t = useTranslations("editor");
  if (!issues.length) return null;
  const hasError = issues.some((i) => i.severity === "error");
  const Icon = hasError ? CircleX : CircleAlert;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button type="button" className="inline-flex items-center gap-1" aria-label={t("issuesTooltip", { count: issues.length })}>
          <Icon className={cn("size-3.5", hasError ? "text-destructive" : "text-warning")} />
          <span className={cn("font-mono text-[11px]", hasError ? "text-destructive" : "text-warning")}>{issues.length}</span>
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">
        <ul className="space-y-1">
          {issues.map((i, n) => (
            <li key={n}>
              <span className="font-mono opacity-70">{i.code}</span> {i.message}
            </li>
          ))}
        </ul>
      </TooltipContent>
    </Tooltip>
  );
}

function TranslationCell(props: {
  locale: string;
  translation: Translation;
  active: boolean;
  editing: boolean;
  draft: string;
  saving: boolean;
  onDraft: (v: string) => void;
  onFocus: () => void;
  onEdit: () => void;
  onCancel: () => void;
  onSave: () => void;
  onApprove: () => void;
}) {
  const t = useTranslations("editor");
  const { translation: tr, locale } = props;

  if (props.editing) {
    return (
      <div role="gridcell" className="px-2 py-2">
        <Textarea
          autoFocus
          dir="auto"
          lang={locale}
          value={props.draft}
          disabled={props.saving}
          onChange={(e) => props.onDraft(e.target.value)}
          onFocus={(e) => e.currentTarget.setSelectionRange(e.currentTarget.value.length, e.currentTarget.value.length)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              props.onSave();
            } else if (e.key === "Escape") {
              e.preventDefault();
              props.onCancel();
            }
          }}
          aria-label={t("editPlaceholder", { locale })}
          placeholder={t("editPlaceholder", { locale })}
          className="min-h-16 text-sm"
        />
      </div>
    );
  }

  return (
    <div
      role="gridcell"
      tabIndex={-1}
      aria-selected={props.active}
      onClick={props.onFocus}
      onDoubleClick={props.onEdit}
      className={cn(
        "group/cell relative min-w-0 cursor-text px-3 py-2.5",
        props.active && "ring-2 ring-ring/60 ring-inset",
        tr.status === "missing" && "bg-destructive/[0.03]",
      )}
    >
      {tr.value ? (
        <p dir="auto" lang={locale} className="line-clamp-3 whitespace-pre-wrap">{tr.value}</p>
      ) : (
        <p className="text-muted-foreground italic">{t("emptyValue")}</p>
      )}
      <div className="mt-1.5 flex items-center gap-2">
        <StatusBadge status={tr.status} />
        {tr.score !== null && (
          <span
            className={cn("font-mono text-[11px] tabular-nums", tr.score < 80 ? "text-warning" : "text-muted-foreground")}
            title={t("score", { score: tr.score })}
          >
            {tr.score}
          </span>
        )}
        <IssuesIcon issues={tr.issues} />
        {(tr.status === "machine" || tr.status === "stale") && tr.value && (
          <Button
            variant="ghost"
            size="icon-xs"
            className="ms-auto opacity-0 group-hover/cell:opacity-100 focus-visible:opacity-100"
            aria-label={t("approve")}
            onClick={(e) => {
              e.stopPropagation();
              props.onApprove();
            }}
          >
            <Check />
          </Button>
        )}
      </div>
    </div>
  );
}

function DetailPanel({ slug, project, row, onClose }: { slug: string; project: Project; row?: KeyRow; onClose: () => void }) {
  const t = useTranslations("editor.detail");
  const te = useTranslations("editor");
  const tc = useTranslations("common");
  const to = useTranslations("editor.origin");
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  const glossary = useQuery({ queryKey: qk.glossary(slug), queryFn: () => api.glossary(slug) });
  const history = useQuery({
    queryKey: qk.history(slug, row?.key ?? ""),
    queryFn: () => api.history(slug, row!.key),
    enabled: !!row,
  });

  const terms = useMemo(() => {
    if (!row) return [];
    return (glossary.data ?? []).filter((g) =>
      g.case_sensitive ? row.source.includes(g.term) : row.source.toLowerCase().includes(g.term.toLowerCase()),
    );
  }, [glossary.data, row]);

  const section = "border-b px-4 py-4";
  const h = "mb-2 text-xs font-medium text-muted-foreground";

  return (
    <div className="text-sm">
      <div className="sticky top-0 z-10 flex items-center justify-between border-b bg-background px-4 py-2.5">
        <h2 className="font-medium">{t("title")}</h2>
        <Button variant="ghost" size="xs" onClick={onClose}>
          {tc("close")}
        </Button>
      </div>
      {!row ? (
        <p className="p-6 text-center text-muted-foreground">{t("selectKey")}</p>
      ) : (
        <>
          <div className={section}>
            <p className="font-mono text-xs break-all">{row.key}</p>
            <p dir="auto" lang={project.source_locale} className="mt-2 whitespace-pre-wrap">{row.source}</p>
          </div>
          <div className={section}>
            <h3 className={h}>{t("description")}</h3>
            <p className={row.description ? "" : "text-muted-foreground"}>{row.description || t("noDescription")}</p>
          </div>
          <div className={section}>
            <h3 className={h}>{t("references")}</h3>
            {row.refs.length ? (
              <ul className="space-y-1">
                {row.refs.map((r) => (
                  <li key={`${r.path}:${r.line}`} dir="ltr" className="flex items-center gap-1.5 font-mono text-xs">
                    <FileCode className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate">{r.path}</span>
                    <span className="text-muted-foreground">:{r.line}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-muted-foreground">{t("noReferences")}</p>
            )}
          </div>
          <div className={section}>
            <h3 className={h}>{t("translations")}</h3>
            <ul className="space-y-3">
              {project.target_locales.map((l) => {
                const x = tr(row, l);
                return (
                  <li key={l}>
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-xs">{l}</span>
                      <StatusBadge status={x.status} />
                      {x.score !== null && <span className="font-mono text-[11px] text-muted-foreground">{te("score", { score: x.score })}</span>}
                      {x.model && <span className="ms-auto truncate font-mono text-[11px] text-muted-foreground">{x.model}</span>}
                    </div>
                    {x.value && <p dir="auto" lang={l} className="mt-1 whitespace-pre-wrap">{x.value}</p>}
                    {x.issues.map((i, n) => (
                      <p key={n} className={cn("mt-1 flex gap-1.5 text-xs", i.severity === "error" ? "text-destructive" : "text-warning")}>
                        {i.severity === "error" ? <CircleX className="mt-0.5 size-3.5 shrink-0" /> : <CircleAlert className="mt-0.5 size-3.5 shrink-0" />}
                        {i.message}
                      </p>
                    ))}
                  </li>
                );
              })}
            </ul>
          </div>
          <div className={section}>
            <h3 className={h}>{t("glossary")}</h3>
            {terms.length ? (
              <ul className="space-y-2">
                {terms.map((g) => (
                  <li key={g.id}>
                    <span className="font-medium">{g.term}</span>
                    {g.do_not_translate ? (
                      <span className="ms-2 text-xs text-muted-foreground">{t("doNotTranslate")}</span>
                    ) : (
                      <span className="ms-2 text-xs text-muted-foreground">
                        {Object.entries(g.translations).map(([l, v]) => `${l}: ${v}`).join(" · ")}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-muted-foreground">{t("noGlossary")}</p>
            )}
          </div>
          <div className="px-4 py-4">
            <h3 className={h}>{t("history")}</h3>
            {history.isPending ? (
              <Skeleton className="h-16" />
            ) : !history.data?.length ? (
              <p className="text-muted-foreground">{t("noHistory")}</p>
            ) : (
              <ol className="space-y-3 border-s ps-3">
                {history.data.map((e, n) => (
                  <li key={n}>
                    <div className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
                      <span className="font-mono text-foreground">{e.locale}</span>
                      <span>{isOrigin(e.origin) ? to(e.origin) : e.origin}</span>
                      {e.model && <span className="font-mono">{e.model}</span>}
                      {e.author && <span>{t("by", { author: e.author })}</span>}
                      <span className="ms-auto">{format.relativeTime(new Date(e.created_at), now)}</span>
                    </div>
                    <p dir="auto" lang={e.locale} className="mt-0.5 whitespace-pre-wrap">{e.value}</p>
                  </li>
                ))}
              </ol>
            )}
          </div>
        </>
      )}
    </div>
  );
}
