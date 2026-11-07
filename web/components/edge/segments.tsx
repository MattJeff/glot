"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Inbox, Loader2 } from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, StatusBadge, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import type { Issue, Project } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { type EdgeSegment, edgeApi, edgeKeys, SEGMENT_STATUSES, type SegmentStatus, splitContext } from "./api";

const PER_PAGE = 50;
// ponytail: the server returns up to 1000 segments (busiest first); search and paging are client-side.
const LIMIT = 1000;

/**
 * Edge segments (rendered HTML text, no key) next to the key editor: filter, edit, approve.
 * `queue` = the review queue: only segments pending human review (sensitive pages).
 */
export function SegmentsEditor({ project, queue = false }: { project: Project; queue?: boolean }) {
  const t = useTranslations("edge.segments");
  const name = useLocaleName();
  const params = useSearchParams();
  const [locale, setLocale] = useState(params.get("locale") || project.target_locales[0] || "");
  const [status, setStatus] = useState<SegmentStatus | "all">(queue ? "pending" : "all");
  const [q, setQ] = useState(queue ? "" : (params.get("q") ?? ""));
  const [page, setPage] = useState(1);
  const query = { locale, status: status === "all" ? undefined : status, limit: LIMIT };
  const list = useQuery({
    queryKey: [...edgeKeys.segments(project.slug), query],
    queryFn: () => edgeApi.segments(project.slug, query),
    enabled: !!locale,
    placeholderData: (prev) => prev,
  });
  const needle = q.trim().toLowerCase();
  const items = (list.data?.segments ?? []).filter(
    (s) => !needle || s.source.toLowerCase().includes(needle) || (s.value ?? "").toLowerCase().includes(needle),
  );
  const pages = Math.max(1, Math.ceil(items.length / PER_PAGE));
  const shown = items.slice((page - 1) * PER_PAGE, page * PER_PAGE);

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{queue ? t("queueIntro") : t("intro")}</p>
      <div className="flex flex-wrap gap-2">
        <Input
          type="search"
          aria-label={t("search")}
          placeholder={t("search")}
          value={q}
          onChange={(e) => {
            setQ(e.target.value);
            setPage(1);
          }}
          className="min-w-56 flex-1"
        />
        <Select
          value={locale}
          onValueChange={(v) => {
            setLocale(v);
            setPage(1);
          }}
        >
          <SelectTrigger aria-label={t("locale")} className="w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {project.target_locales.map((l) => (
              <SelectItem key={l} value={l}>
                {name(l)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {!queue && (
          <Select
            value={status}
            onValueChange={(v) => {
              setStatus(v as SegmentStatus | "all");
              setPage(1);
            }}
          >
            <SelectTrigger aria-label={t("status")} className="w-44">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("all")}</SelectItem>
              {SEGMENT_STATUSES.map((s) => (
                <SelectItem key={s} value={s}>
                  {t(`statuses.${s}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>

      {list.isPending ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : shown.length === 0 ? (
        <EmptyState icon={Inbox} title={queue ? t("queueEmpty.title") : t("empty.title")} body={queue ? t("queueEmpty.body") : t("empty.body")} />
      ) : (
        <>
          <ul className="divide-y rounded-xl border">
            {shown.map((s) => (
              <SegmentRow key={`${s.hash}|${s.context}|${locale}|${s.value ?? ""}`} slug={project.slug} segment={s} locale={locale} />
            ))}
          </ul>
          {pages > 1 && (
            <div className="flex items-center justify-end gap-2 text-sm">
              <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage(page - 1)}>
                {t("prev")}
              </Button>
              <span className="tabular-nums text-muted-foreground">
                {page} / {pages}
              </span>
              <Button size="sm" variant="outline" disabled={page >= pages} onClick={() => setPage(page + 1)}>
                {t("next")}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function SegmentBadge({ status }: { status: SegmentStatus }) {
  const t = useTranslations("edge.segments.statuses");
  if (status === "pending")
    return (
      <Badge variant="secondary" className="h-5 bg-warning/15 px-1.5 text-[11px] text-warning-foreground">
        {t("pending")}
      </Badge>
    );
  return <StatusBadge status={status} />;
}

function SegmentRow({ slug, segment: s, locale }: { slug: string; segment: EdgeSegment; locale: string }) {
  const t = useTranslations("edge.segments");
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  const qc = useQueryClient();
  const message = useErrorMessage();
  const [value, setValue] = useState(s.value ?? "");
  const [issues, setIssues] = useState<Issue[]>(s.issues ?? []);
  // Saving a human translation approves it (status `reviewed`): it is served from the next request.
  const save = useMutation({
    mutationFn: () => edgeApi.putSegment(slug, s.hash, locale, { context: s.context, value }),
    onSuccess: (r) => {
      setIssues(r.issues);
      qc.invalidateQueries({ queryKey: edgeKeys.all(slug) });
      toast.success(t("approved"));
    },
    onError: (e) => toast.error(message(e)),
  });
  const ctx = splitContext(s.context);
  const changed = value !== (s.value ?? "");

  return (
    <li className="grid gap-3 p-4 md:grid-cols-2">
      <div className="min-w-0 space-y-1.5">
        <p className="text-sm text-pretty break-words">{s.source}</p>
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <Badge variant="outline" className="font-mono" translate="no">
            {ctx.role || "—"}
          </Badge>
          <span className="font-mono" translate="no" dir="ltr">
            {ctx.template}
          </span>
          {s.needs_review && <Badge variant="outline">{t("sensitive")}</Badge>}
          <span className="tabular-nums">· {t("hits", { n: s.hits })}</span>
          <span>· {format.relativeTime(new Date(s.last_seen), now)}</span>
        </div>
        {s.url_sample && (
          <span dir="ltr" translate="no" className="block truncate font-mono text-xs text-muted-foreground">
            {s.url_sample}
          </span>
        )}
      </div>
      <div className="min-w-0 space-y-2">
        <Textarea aria-label={t("translation")} rows={2} value={value} onChange={(e) => setValue(e.target.value)} lang={locale} dir="auto" />
        {issues.length > 0 && (
          <ul className="space-y-0.5 text-xs">
            {issues.map((i, k) => (
              <li key={k} className={i.severity === "error" ? "text-destructive" : "text-warning-foreground"}>
                {i.message}
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-2">
          <SegmentBadge status={s.status} />
          {(changed || s.status !== "reviewed") && (
            <Button size="sm" className="ms-auto" disabled={save.isPending || !value.trim()} onClick={() => save.mutate()}>
              {save.isPending ? <Loader2 className="animate-spin" /> : <Check />}
              {changed ? t("save") : t("approve")}
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}
