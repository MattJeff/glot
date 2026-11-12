"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, Pencil, RefreshCw, Search, TriangleAlert, X } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, PageHeader } from "@/components/common";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useSlug } from "@/lib/hooks";
import { type LocalSeo, localSeo, parseTerms, type SeoContent, type SeoIssue, type SeoMarket, type SeoRow, type SeoWindow } from "@/lib/local-seo";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";

/** Same unit as the server's validators: CJK, Hangul and fullwidth glyphs count double. */
const width = (s: string) => [...s].reduce((n, c) => n + (/[ᄀ-ᅟ⺀-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]/.test(c) ? 2 : 1), 0);

const STATUS_STYLE: Record<SeoRow["status"], string> = {
  proposed: "bg-info/12 text-info-foreground",
  approved: "bg-success/12 text-success-foreground",
  auto: "bg-success/12 text-success-foreground",
  rejected: "bg-muted text-muted-foreground",
  stale: "bg-warning/15 text-warning-foreground",
  needs_demand: "bg-muted text-muted-foreground",
};

export default function LocalSeoPage() {
  const t = useTranslations("localSeo");
  const slug = useSlug();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const data = useQuery({ queryKey: localSeo.key(slug), queryFn: () => localSeo.get(slug) });
  const [editing, setEditing] = useState<SeoRow | null>(null);
  const [keywords, setKeywords] = useState<SeoMarket | null>(null);
  const [confirmAuto, setConfirmAuto] = useState(false);
  const [warnings, setWarnings] = useState<string[]>([]);
  const refresh = () => qc.invalidateQueries({ queryKey: localSeo.key(slug) });
  const onError = (e: unknown) => toast.error(message(e));

  // Back from Google's consent screen: say what happened, once.
  useEffect(() => {
    const url = new URL(window.location.href);
    const code = url.searchParams.get("gsc");
    if (!code) return;
    if (code === "connected") toast.success(t("gsc.result.connected"));
    else toast.error(t(`gsc.result.${code === "denied" || code === "no_property" ? code : "failed"}`));
    url.searchParams.delete("gsc");
    window.history.replaceState(null, "", url);
  }, [t]);

  const scan = useMutation({
    mutationFn: (regenerate: boolean) => localSeo.scan(slug, { regenerate }),
    onSuccess: (r) => {
      setWarnings(r.warnings);
      const proposed = r.results.filter((x) => x.status === "proposed" || x.status === "auto").length;
      toast.success(t("scan.done", { pages: r.pages, proposed }));
      refresh();
    },
    onError,
  });
  const mode = useMutation({
    mutationFn: (m: "review" | "auto") => localSeo.settings(slug, { mode: m }),
    onSuccess: () => {
      setConfirmAuto(false);
      refresh();
    },
    onError,
  });
  const decide = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "approve" | "reject" }) => localSeo[action](slug, id),
    onSuccess: (_, v) => {
      toast.success(t(v.action === "approve" ? "approved" : "rejected"));
      refresh();
    },
    onError,
  });
  const connect = useMutation({
    mutationFn: () => localSeo.gscConnect(slug),
    onSuccess: (r) => window.location.assign(r.url),
    onError,
  });
  const disconnect = useMutation({ mutationFn: () => localSeo.gscDisconnect(slug), onSuccess: refresh, onError });
  const metrics = useMutation({
    mutationFn: () => localSeo.refreshMetrics(slug),
    onSuccess: (r) => {
      setWarnings(r.errors);
      toast.success(t("metrics.refreshed", { days: r.days }));
      refresh();
    },
    onError,
  });

  const d = data.data;
  const scanButton = (
    <Button onClick={() => scan.mutate(false)} disabled={scan.isPending || (d ? !d.site.origin : true)}>
      {scan.isPending ? <Loader2 className="animate-spin" /> : <Search />} {scan.isPending ? t("scan.running") : t("scan.action")}
    </Button>
  );
  const paths = d ? [...new Set(d.rows.map((r) => r.path))] : [];

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader title={t("title")} subtitle={t("subtitle")} actions={d?.rows.length ? scanButton : null} />
      {data.isPending ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : data.isError ? (
        <ErrorState error={data.error} onRetry={() => data.refetch()} />
      ) : (
        d && (
          <div className="space-y-6">
            <Sources
              d={d}
              slug={slug}
              onSaved={refresh}
              onConnect={() => connect.mutate()}
              onDisconnect={() => disconnect.mutate()}
              onRefreshMetrics={() => metrics.mutate()}
              busy={connect.isPending || disconnect.isPending || metrics.isPending}
              onMode={(auto) => (auto ? setConfirmAuto(true) : mode.mutate("review"))}
              onKeywords={setKeywords}
            />
            {warnings.length > 0 && (
              <div role="status" className="rounded-xl border border-warning/40 bg-warning/10 px-4 py-3 text-sm">
                <div className="mb-1 flex items-center justify-between gap-2 font-medium">
                  <span className="flex items-center gap-2">
                    <TriangleAlert className="size-4" aria-hidden /> {t("warnings")}
                  </span>
                  <Button variant="ghost" size="icon-sm" aria-label={t("dismiss")} onClick={() => setWarnings([])}>
                    <X />
                  </Button>
                </div>
                <ul className="list-disc space-y-0.5 ps-5">
                  {warnings.map((w) => (
                    <li key={w}>{w}</li>
                  ))}
                </ul>
              </div>
            )}
            {d.rows.length === 0 ? (
              <EmptyState icon={Search} title={t("empty.title")} body={d.site.origin ? t("empty.body", { site: d.site.origin }) : t("empty.noSite")} action={d.site.origin ? scanButton : null} />
            ) : (
              paths.map((path) => (
                <section key={path} className="overflow-hidden rounded-xl border bg-card shadow-(--shadow-card)">
                  <header className="border-b bg-muted/40 px-4 py-2.5">
                    <h2 className="font-mono text-sm font-medium" dir="ltr">
                      {path}
                    </h2>
                    <p className="mt-0.5 truncate text-xs text-muted-foreground" dir="auto">
                      {d.rows.find((r) => r.path === path)?.source.title}
                    </p>
                  </header>
                  <div className="divide-y">
                    {d.rows
                      .filter((r) => r.path === path)
                      .map((r) => (
                        <Row
                          key={r.id}
                          row={r}
                          market={d.markets.find((m) => m.locale === r.locale)}
                          gsc={d.gsc.connected}
                          busy={decide.isPending}
                          onApprove={() => decide.mutate({ id: r.id, action: "approve" })}
                          onReject={() => decide.mutate({ id: r.id, action: "reject" })}
                          onEdit={() => setEditing(r)}
                        />
                      ))}
                  </div>
                </section>
              ))
            )}
          </div>
        )
      )}

      {editing && <EditDialog slug={slug} row={editing} market={d?.markets.find((m) => m.locale === editing.locale)} onClose={() => setEditing(null)} onSaved={refresh} />}
      {keywords && d && <KeywordsDialog slug={slug} market={keywords} data={d} onClose={() => setKeywords(null)} onSaved={refresh} />}

      <AlertDialog open={confirmAuto} onOpenChange={setConfirmAuto}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("auto.confirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("auto.confirmBody")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => mode.mutate("auto")}>{t("auto.confirm")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Where the data comes from: the site, Search Console, the customer's keywords, and the consent mode. */
function Sources({
  d,
  slug,
  busy,
  onSaved,
  onConnect,
  onDisconnect,
  onRefreshMetrics,
  onMode,
  onKeywords,
}: {
  d: LocalSeo;
  slug: string;
  busy: boolean;
  onSaved: () => void;
  onConnect: () => void;
  onDisconnect: () => void;
  onRefreshMetrics: () => void;
  onMode: (auto: boolean) => void;
  onKeywords: (m: SeoMarket) => void;
}) {
  const t = useTranslations("localSeo");
  const message = useErrorMessage();
  const [site, setSite] = useState("");
  const save = useMutation({
    mutationFn: () => localSeo.settings(slug, { site_url: site }),
    onSuccess: onSaved,
    onError: (e) => toast.error(message(e)),
  });
  const card = "rounded-xl border bg-card p-4 shadow-(--shadow-card)";
  return (
    <div className="grid gap-4 md:grid-cols-3">
      <div className={card}>
        <h2 className="text-sm font-medium">{t("site.title")}</h2>
        {d.site.origin ? (
          <p className="mt-1 truncate font-mono text-xs text-muted-foreground" dir="ltr">
            {d.site.origin}
          </p>
        ) : (
          <form
            className="mt-2 space-y-2"
            onSubmit={(e) => {
              e.preventDefault();
              save.mutate();
            }}
          >
            <p className="text-xs text-muted-foreground">{t("site.missing")}</p>
            <div className="flex gap-2">
              <Input aria-label={t("site.title")} type="url" required placeholder="https://example.com" value={site} onChange={(e) => setSite(e.target.value)} />
              <Button type="submit" variant="outline" disabled={save.isPending}>
                {t("save")}
              </Button>
            </div>
          </form>
        )}
        <div className="mt-3 flex items-center justify-between gap-3 border-t pt-3">
          <Label htmlFor="local-seo-auto" className="text-xs font-normal text-muted-foreground">
            {d.mode === "auto" ? t("auto.on") : t("auto.off")}
          </Label>
          <Switch id="local-seo-auto" checked={d.mode === "auto"} onCheckedChange={onMode} />
        </div>
      </div>

      <div className={card}>
        <h2 className="text-sm font-medium">{t("gsc.title")}</h2>
        {d.gsc.connected ? (
          <>
            <p className="mt-1 truncate font-mono text-xs text-muted-foreground" dir="ltr">
              {d.gsc.site_url}
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={onRefreshMetrics} disabled={busy}>
                <RefreshCw /> {t("metrics.refresh")}
              </Button>
              <Button variant="ghost" size="sm" onClick={onDisconnect} disabled={busy}>
                {t("gsc.disconnect")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <p className="mt-1 text-xs text-pretty text-muted-foreground">{d.gsc.configured ? t("gsc.notConnected") : t("gsc.notConfigured")}</p>
            {d.gsc.configured && (
              <Button className="mt-3" variant="outline" size="sm" onClick={onConnect} disabled={busy}>
                {t("gsc.connect")}
              </Button>
            )}
          </>
        )}
      </div>

      <div className={card}>
        <h2 className="text-sm font-medium">{t("keywords.title")}</h2>
        <p className="mt-1 text-xs text-pretty text-muted-foreground">{t("keywords.hint")}</p>
        <div className="mt-3 flex flex-wrap gap-1.5">
          {d.markets.map((m) => {
            const n = d.manual_demand.filter((x) => x.locale === m.locale).length;
            return (
              <Button key={m.locale} variant="outline" size="sm" onClick={() => onKeywords(m)}>
                <span className="font-mono">{m.locale}</span>
                <span className="text-muted-foreground">{n}</span>
              </Button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function Issues({ issues }: { issues: SeoIssue[] }) {
  if (!issues.length) return null;
  return (
    <ul className="mt-2 space-y-1 text-xs">
      {issues.map((i, n) => (
        <li key={n} className={cn("flex gap-1.5", i.severity === "error" ? "text-destructive" : "text-warning-foreground")}>
          <TriangleAlert className="mt-0.5 size-3 shrink-0" aria-hidden />
          <span>{i.message}</span>
        </li>
      ))}
    </ul>
  );
}

function Snippet({ label, content, lang, limits }: { label: string; content: SeoContent; lang: string; limits?: SeoMarket["limits"] }) {
  const t = useTranslations("localSeo");
  const count = (s: string | null, max?: number) =>
    s && max ? <span className={cn("ms-1.5 font-mono text-[10px]", width(s) > max ? "text-destructive" : "text-muted-foreground")}>{width(s)}/{max}</span> : null;
  return (
    <div className="min-w-0">
      <div className="mb-1 text-xs font-medium text-muted-foreground">{label}</div>
      {content.title || content.description || content.h1 ? (
        <div lang={lang} dir="auto" className="space-y-1 text-sm">
          {content.title && (
            <p className="font-medium text-pretty">
              {content.title}
              {count(content.title, limits?.title)}
            </p>
          )}
          {content.description && (
            <p className="text-pretty text-muted-foreground">
              {content.description}
              {count(content.description, limits?.description)}
            </p>
          )}
          {content.h1 && (
            <p className="text-xs">
              <span className="font-mono text-muted-foreground">h1</span> {content.h1}
            </p>
          )}
          {content.slug && (
            <p className="text-xs">
              <span className="font-mono text-muted-foreground">{t("slug")}</span> <span dir="ltr">/{content.slug}</span>
            </p>
          )}
          {content.faq.map((f) => (
            <p key={f.question} className="text-xs">
              <span className="font-medium">{f.question}</span> <span className="text-muted-foreground">{f.answer}</span>
            </p>
          ))}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">—</p>
      )}
    </div>
  );
}

function Metrics({ row, gsc }: { row: SeoRow; gsc: boolean }) {
  const t = useTranslations("localSeo");
  const f = useFormatter();
  if (!row.metrics) return null;
  const { before, after } = row.metrics;
  if (!before && !after) return <p className="mt-2 text-xs text-muted-foreground">{gsc ? t("metrics.none") : t("metrics.notConnected")}</p>;
  const cell = (w: SeoWindow | null) =>
    w ? t("metrics.window", { days: w.days, impressions: f.number(w.impressions), clicks: f.number(w.clicks), position: w.position === null ? "—" : f.number(w.position, { maximumFractionDigits: 1 }) }) : t("metrics.noRows");
  return (
    <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
      <dt className="text-muted-foreground">{t("metrics.before")}</dt>
      <dd>{cell(before)}</dd>
      <dt className="text-muted-foreground">{t("metrics.after")}</dt>
      <dd>{cell(after)}</dd>
    </dl>
  );
}

function Row({
  row,
  market,
  gsc,
  busy,
  onApprove,
  onReject,
  onEdit,
}: {
  row: SeoRow;
  market?: SeoMarket;
  gsc: boolean;
  busy: boolean;
  onApprove: () => void;
  onReject: () => void;
  onEdit: () => void;
}) {
  const t = useTranslations("localSeo");
  const f = useFormatter();
  const live = row.status === "approved" || row.status === "auto";
  const hasProposal = !!(row.effective.title || row.effective.description || row.effective.h1);
  const blocked = row.issues.some((i) => i.severity === "error");
  return (
    <div className="grid gap-4 px-4 py-4 lg:grid-cols-[9rem_1fr_1fr]">
      <div>
        <div className="font-mono text-sm font-medium">{row.locale}</div>
        {market && <div className="text-xs text-muted-foreground">{t(`engine.${market.engine}`)}</div>}
        <span className={cn("mt-2 inline-block rounded-md px-1.5 py-0.5 text-xs font-medium", STATUS_STYLE[row.status])}>{t(`status.${row.status}`)}</span>
      </div>

      <div className="min-w-0">
        <Snippet label={t("current")} content={row.current} lang={row.locale} limits={market?.limits} />
        <Issues issues={row.audit} />
      </div>

      <div className="min-w-0">
        {hasProposal ? (
          <>
            <Snippet label={row.edited ? t("edited") : t("proposed")} content={row.effective} lang={row.locale} limits={market?.limits} />
            <Issues issues={row.issues} />
          </>
        ) : (
          <>
            <div className="mb-1 text-xs font-medium text-muted-foreground">{t("proposed")}</div>
            <p className="text-sm text-pretty text-muted-foreground">{row.status === "needs_demand" && row.terms.length === 0 ? t("needsDemand", { locale: row.locale }) : row.rationale}</p>
          </>
        )}
        {row.terms.length > 0 && (
          <div className="mt-3">
            <div className="mb-1 text-xs font-medium text-muted-foreground">{t("why")}</div>
            <ul className="flex flex-wrap gap-1.5">
              {row.terms.slice(0, 8).map((term) => (
                <li key={term.term}>
                  <Badge variant="outline" className="gap-1.5 font-normal">
                    <span lang={row.locale} dir="auto">
                      {term.term}
                    </span>
                    <span className="text-muted-foreground">
                      {term.impressions === null ? t("volumeUnknown") : f.number(term.impressions)} · {t(`source.${term.source}`)}
                    </span>
                  </Badge>
                </li>
              ))}
            </ul>
            {hasProposal && row.rationale && <p className="mt-1.5 text-xs text-pretty text-muted-foreground">{row.rationale}</p>}
          </div>
        )}
        <Metrics row={row} gsc={gsc} />
        {(hasProposal || row.terms.length > 0) && (
          <div className="mt-3 flex flex-wrap gap-2">
            {hasProposal && !live && (
              <Button size="sm" onClick={onApprove} disabled={busy || blocked} title={blocked ? t("blocked") : undefined}>
                <Check /> {t("approve")}
              </Button>
            )}
            <Button size="sm" variant="outline" onClick={onEdit}>
              <Pencil /> {t("edit")}
            </Button>
            {hasProposal && row.status !== "rejected" && (
              <Button size="sm" variant="ghost" onClick={onReject} disabled={busy}>
                <X /> {live ? t("takeDown") : t("reject")}
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function EditDialog({ slug, row, market, onClose, onSaved }: { slug: string; row: SeoRow; market?: SeoMarket; onClose: () => void; onSaved: () => void }) {
  const t = useTranslations("localSeo");
  const message = useErrorMessage();
  const [c, setC] = useState<SeoContent>(row.effective);
  const [issues, setIssues] = useState<SeoIssue[]>(row.issues);
  const save = useMutation({
    mutationFn: (approve: boolean) => localSeo.edit(slug, row.id, c, approve),
    onSuccess: (r, approve) => {
      onSaved();
      setIssues(r.issues);
      if (approve || !r.issues.length) {
        toast.success(t(approve ? "approved" : "saved"));
        onClose();
      }
    },
    onError: (e) => toast.error(message(e)),
  });
  const field = (key: "title" | "description" | "h1" | "slug", max?: number, area = false) => {
    const value = c[key] ?? "";
    const props = {
      id: `seo-${key}`,
      value,
      lang: key === "slug" ? undefined : row.locale,
      dir: key === "slug" ? ("ltr" as const) : ("auto" as const),
      onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setC({ ...c, [key]: e.target.value || null }),
    };
    return (
      <div className="space-y-1.5">
        <div className="flex items-baseline justify-between">
          <Label htmlFor={props.id}>{t(`field.${key}`)}</Label>
          {max && <span className={cn("font-mono text-xs", width(value) > max ? "text-destructive" : "text-muted-foreground")}>{width(value)}/{max}</span>}
        </div>
        {area ? <Textarea {...props} rows={3} /> : <Input {...props} />}
      </div>
    );
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("editTitle", { path: row.path, locale: row.locale })}</DialogTitle>
          <DialogDescription>{t("editBody")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {field("title", market?.limits.title)}
          {field("description", market?.limits.description, true)}
          {field("h1")}
          {field("slug")}
          <Issues issues={issues} />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button variant="outline" onClick={() => save.mutate(false)} disabled={save.isPending}>
            {t("saveDraft")}
          </Button>
          <Button onClick={() => save.mutate(true)} disabled={save.isPending}>
            {t("saveApprove")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function KeywordsDialog({ slug, market, data, onClose, onSaved }: { slug: string; market: SeoMarket; data: LocalSeo; onClose: () => void; onSaved: () => void }) {
  const t = useTranslations("localSeo");
  const message = useErrorMessage();
  const [text, setText] = useState(() =>
    data.manual_demand
      .filter((x) => x.locale === market.locale && x.path === "")
      .map((x) => (x.impressions === null ? x.term : `${x.term}, ${x.impressions}`))
      .join("\n"),
  );
  const save = useMutation({
    mutationFn: () => localSeo.demand(slug, { locale: market.locale, terms: parseTerms(text) }),
    onSuccess: (r) => {
      toast.success(t("keywords.saved", { count: r.saved }));
      onSaved();
      onClose();
    },
    onError: (e) => toast.error(message(e)),
  });
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("keywords.dialogTitle", { locale: market.locale })}</DialogTitle>
          <DialogDescription>{t("keywords.dialogBody")}</DialogDescription>
        </DialogHeader>
        <Textarea aria-label={t("keywords.title")} rows={10} lang={market.locale} dir="auto" value={text} onChange={(e) => setText(e.target.value)} />
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button onClick={() => save.mutate()} disabled={save.isPending}>
            {t("save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
