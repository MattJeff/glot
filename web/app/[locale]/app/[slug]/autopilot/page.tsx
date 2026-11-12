"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Check, Loader2, Lock, Rocket, Sparkles, Square, TrendingUp } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, EmptyState, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { useSigned, VerdictBadge } from "@/components/launch-story";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import {
  api,
  qk,
  type AutopilotOpening,
  type AutopilotOpportunity,
  type AutopilotOptimization,
  type AutopilotOverview,
  type AutopilotSettings,
  type Interval,
} from "@/lib/api";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";
import { Section } from "../insights/parts";
import { PosthogSource } from "./posthog";

/** Same cap as the server (`autopilot::budget_cap`): the dry-run estimate + 50 %, at least 50 cents. */
const cap = (cost: number) => Math.max(0.5, Math.ceil(cost * 1.5 * 100) / 100);

function useFmt(d?: AutopilotOverview) {
  const format = useFormatter();
  const currency = d?.value?.currency ?? d?.settings.currency ?? "USD";
  return {
    money: (n: number, cur = currency) => format.number(n, { style: "currency", currency: cur, maximumFractionDigits: Math.abs(n) < 10 ? 2 : 0 }),
    pct: (n: number) => format.number(n, { style: "percent", maximumFractionDigits: n > 0 && n < 0.1 ? 1 : 0 }),
    num: (n: number, sign = false) => format.number(n, { maximumFractionDigits: Math.abs(n) < 10 ? 1 : 0, signDisplay: sign ? "exceptZero" : "auto" }),
  };
}

export default function AutopilotPage() {
  const t = useTranslations("autopilot");
  const slug = useSlug();
  const origin = useOrigin() ?? "";
  const project = useProject(slug);
  // undefined = let the server use the source with more visits.
  const [source, setSource] = useState<"insights" | "posthog" | undefined>();
  const q = useQuery({
    queryKey: [...qk.autopilot(slug), source],
    queryFn: () => api.autopilot(slug, { source }),
    // Translation progress and test counters move on their own.
    refetchInterval: (query) => (query.state.data?.openings.some((o) => o.status === "translating") ? 4000 : 60_000),
  });
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const d = q.data;
  return (
    <div className="mx-auto max-w-5xl">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button size="sm" variant="outline" asChild>
            <Link href="/docs/autopilot">
              <BookOpen /> {t("docs")}
            </Link>
          </Button>
        }
      />
      {!d ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : (
        <div className="grid gap-6">
          {d.total_visits === 0 ? (
            <EmptyState icon={TrendingUp} title={t("empty.title")} body={t("empty.body")}>
              <CodeBlock
                code={`<script src="https://unpkg.com/@glot-i18n/insights/dist/insights.global.js" data-key="${d.public_key}" data-endpoint="${origin}" defer></script>`}
              />
              <p className="mt-4 mb-2 text-sm text-muted-foreground">{t("empty.track")}</p>
              <CodeBlock code={`glot.track("${d.settings.goal}", { value: 49, currency: "${d.settings.currency}" })`} />
              {!d.posthog.connected && <p className="mt-4 text-sm text-muted-foreground">{t("empty.posthog")}</p>}
            </EmptyState>
          ) : (
            <>
              <SourceLine d={d} onPick={setSource} />
              <Markets slug={slug} d={d} />
              <Misrouted d={d} />
            </>
          )}
          {d.openings.length > 0 && <Openings slug={slug} d={d} />}
          <Copy slug={slug} d={d} />
          <SettingsForm key={JSON.stringify(d.settings)} slug={slug} d={d} />
          <PosthogSource key={JSON.stringify(d.posthog)} slug={slug} v={d.posthog} sourceLocale={project.data?.source_locale ?? ""} />
          <Section title={t("collected.title")} subtitle={t("collected.body")}>
            <ul className="list-disc space-y-1 ps-5 text-sm text-muted-foreground">
              <li>{t("collected.insights")}</li>
              <li>{t("collected.variants")}</li>
              <li>{t("collected.revenue")}</li>
              <li>{t("collected.posthog")}</li>
            </ul>
          </Section>
        </div>
      )}
    </div>
  );
}

/** "Source: PostHog · days of data: 41", and a switch when both sources have visits. */
function SourceLine({ d, onPick }: { d: AutopilotOverview; onPick: (s: "insights" | "posthog") => void }) {
  const t = useTranslations("autopilot.source");
  const format = useFormatter();
  const other = d.source.visits.find(([k, n]) => k !== d.source.kind && n > 0);
  return (
    <p className="-mb-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      <span>
        {t("label", { source: t(d.source.kind), days: d.source.days })}
        {d.source.synced_at && <> · {t("synced", { date: format.relativeTime(new Date(d.source.synced_at)) })}</>}
      </span>
      {other && (
        <button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={() => onPick(other[0])}>
          {t("switch", { source: t(other[0]) })}
        </button>
      )}
    </p>
  );
}

function ConfidenceBadge({ level }: { level: "insufficient" | "indicative" | "solid" }) {
  const t = useTranslations("autopilot.confidence");
  return <Badge variant={level === "solid" ? "default" : "outline"}>{t(level)}</Badge>;
}

/** "+90 signups per month" or "€3,600 per month", with its range; the money only when a real value is known. */
function useGain(d: AutopilotOverview) {
  const t = useTranslations("autopilot.opp");
  const f = useFmt(d);
  return (gain: Interval, revenue: Interval | null) => ({
    main: revenue ? t("revenue", { mid: f.money(revenue.mid) }) : t("conversions", { mid: f.num(gain.mid, true), goal: d.source.goal }),
    range: revenue ? t("range", { low: f.money(revenue.low), high: f.money(revenue.high) }) : t("range", { low: f.num(gain.low), high: f.num(gain.high) }),
  });
}

function Markets({ slug, d }: { slug: string; d: AutopilotOverview }) {
  const t = useTranslations("autopilot");
  const name = useLocaleName();
  const f = useFmt(d);
  const gain = useGain(d);
  const [confirm, setConfirm] = useState<AutopilotOpportunity | null>(null);
  const title = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

  return (
    <Section title={t("opp.title")} subtitle={t("opp.subtitle")}>
      {!d.conversions_tracked && <p className="mb-4 rounded-lg border bg-muted/40 px-3 py-2 text-sm">{t("noConversions", { goal: d.source.goal || "?" })}</p>}
      {d.opportunities.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("opp.none")}</p>
      ) : (
        <ol className="grid gap-3">
          {d.opportunities.map((o) => {
            const g = o.gain_monthly;
            const thin = o.confidence === "insufficient" || o.native.conversions === 0;
            const shown = g && !thin ? gain(g, o.revenue_monthly) : null;
            return (
              <li key={o.locale} className="rounded-lg border p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 max-w-3xl">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm tabular-nums text-muted-foreground">#{o.rank}</span>
                      <h3 className="font-medium text-balance">
                        {t(o.kind === "missing" ? "opp.headline" : "opp.headlineFinish", {
                          language: title(name(o.locale)),
                          share: f.pct(o.share),
                          coverage: f.pct(o.coverage),
                        })}
                      </h3>
                      <ConfidenceBadge level={o.confidence} />
                    </div>
                    {shown ? (
                      <p className="mt-2">
                        <span className="text-xl font-semibold tabular-nums">{shown.main}</span>{" "}
                        <span className="text-sm text-muted-foreground">{shown.range}</span>
                      </p>
                    ) : (
                      d.conversions_tracked && !o.gain_locked && <p className="mt-2 text-sm font-medium">{t("opp.insufficient")}</p>
                    )}
                    {o.gain_locked && d.conversions_tracked && (
                      <p className="mt-2 inline-flex items-center gap-1 text-sm text-muted-foreground">
                        <Lock className="size-3" aria-hidden /> {t("opp.locked")}
                      </p>
                    )}
                    <ul className="mt-2 space-y-0.5 text-sm text-muted-foreground">
                      {thin && o.visits_missing > 0 && <li>{t("opp.missingVisits", { count: o.visits_missing })}</li>}
                      {thin && d.conversions_tracked && o.native_visits_missing > 0 && <li>{t("opp.missingNative", { count: o.native_visits_missing })}</li>}
                      {d.conversions_tracked && o.conversions_missing > 0 && <li>{t("opp.missingConversions", { count: o.conversions_missing })}</li>}
                      {o.bounce_rate != null && d.bounce_rate_all != null && (
                        <li>{t("opp.bounce", { rate: f.pct(o.bounce_rate), all: f.pct(d.bounce_rate_all) })}</li>
                      )}
                      {!thin && o.conversion_gap != null && (
                        <li>
                          {t(o.conversion_gap > 0.005 ? "opp.gap" : "opp.gapNone", {
                            gap: f.pct(Math.max(o.conversion_gap, 0)),
                            fallback: f.pct(o.fallback.rate),
                            native: f.pct(o.native.rate),
                          })}
                        </li>
                      )}
                      {shown && g && g.low <= 0 && <li>{t("opp.uncertain")}</li>}
                      <li>
                        {t("opp.cost", { cost: f.money(o.cost_usd, "USD"), keys: o.keys_to_translate })}
                        {shown && o.payback_days != null && <> · {o.payback_days < 1 ? t("opp.paybackFast") : t("opp.payback", { days: Math.ceil(o.payback_days) })}</>}
                      </li>
                    </ul>
                    {shown && g && (
                      <details className="mt-2 text-sm text-muted-foreground">
                        <summary className="cursor-pointer select-none underline-offset-4 hover:underline">{t("opp.method")}</summary>
                        <p className="mt-1 tabular-nums">
                          {t("opp.formula", { visits: f.num(o.monthly_visits), native: f.pct(o.native.rate), fallback: f.pct(o.fallback.rate), gain: f.num(g.mid, true) })}
                          {o.revenue_monthly && d.value && <> {t("opp.formulaMoney", { value: f.money(d.value.amount), revenue: f.money(o.revenue_monthly.mid) })}</>}
                        </p>
                        <p className="mt-1">{t("opp.caveat")}</p>
                      </details>
                    )}
                  </div>
                  <Button onClick={() => setConfirm(o)} variant={o.rank === 1 && !thin ? "default" : "outline"}>
                    <Rocket aria-hidden /> {t("opp.open")}
                  </Button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
      <p className="mt-3 text-xs text-muted-foreground">{t("footnote", { visits: d.total_visits, dataDays: d.data_days, days: d.days })}</p>
      {confirm && <OpenDialog slug={slug} d={d} o={confirm} onClose={() => setConfirm(null)} />}
    </Section>
  );
}

function OpenDialog({ slug, d, o, onClose }: { slug: string; d: AutopilotOverview; o: AutopilotOpportunity; onClose: () => void }) {
  const t = useTranslations("autopilot.open");
  const name = useLocaleName();
  const f = useFmt(d);
  const qc = useQueryClient();
  const [mode, setMode] = useState<"holdout" | "before_after">("holdout");
  const open = useMutation({
    mutationFn: () => api.autopilotOpen(slug, { locale: o.locale, mode }),
    onSuccess: () => {
      for (const k of [qk.autopilot(slug), qk.project(slug), qk.jobs(slug)]) qc.invalidateQueries({ queryKey: k });
      toast.success(t("started", { language: name(o.locale) }));
      onClose();
    },
    onError: (e) => toast.error(e.message),
  });
  return (
    <Dialog open onOpenChange={(x) => !x && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("title", { language: name(o.locale) })}</DialogTitle>
          <DialogDescription>{t("body", { keys: o.keys_to_translate, cost: f.money(o.cost_usd, "USD"), cap: f.money(cap(o.cost_usd), "USD") })}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-1.5">
          <Label htmlFor="autopilot-mode">{t("mode")}</Label>
          <Select value={mode} onValueChange={(v) => setMode(v as typeof mode)}>
            <SelectTrigger id="autopilot-mode" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="holdout">{t("holdout")}</SelectItem>
              <SelectItem value="before_after">{t("before_after")}</SelectItem>
            </SelectContent>
          </Select>
          <p className="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">{t(`${mode}Hint`)}</p>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button onClick={() => open.mutate()} disabled={open.isPending}>
            {open.isPending ? <Loader2 className="animate-spin" /> : <Rocket />} {t("confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Misrouted({ d }: { d: AutopilotOverview }) {
  const t = useTranslations("autopilot.misrouted");
  const name = useLocaleName();
  const f = useFmt(d);
  if (d.misrouted.length === 0) return null;
  return (
    <Section title={t("title")} subtitle={t("subtitle")}>
      <ul className="grid gap-3">
        {d.misrouted.map((m) => (
          <li key={m.language} className="rounded-lg border p-4">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="font-medium text-balance">
                {t("headline", { language: name(m.language), share: f.pct(m.share), instead: m.served_instead ? name(m.served_instead) : "?", count: m.elsewhere })}
              </h3>
              <ConfidenceBadge level={m.confidence} />
            </div>
            <ul className="mt-2 space-y-0.5 text-sm text-muted-foreground">
              {m.conversions_missing > 0 || !m.gain_monthly ? (
                <li className="font-medium text-foreground">{t("missing", { count: m.conversions_missing })}</li>
              ) : (
                <>
                  <li>{t("rates", { mismatched: f.pct(m.mismatched.rate), matched: f.pct(m.matched.rate) })}</li>
                  <li>
                    {m.revenue_monthly
                      ? t("gain", { mid: f.money(m.revenue_monthly.mid), goal: d.source.goal, low: f.money(m.revenue_monthly.low), high: f.money(m.revenue_monthly.high) })
                      : t("gain", { mid: f.num(m.gain_monthly.mid, true), goal: d.source.goal, low: f.num(m.gain_monthly.low), high: f.num(m.gain_monthly.high) })}
                  </li>
                </>
              )}
              <li>{t("caveat")}</li>
            </ul>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Openings({ slug, d }: { slug: string; d: AutopilotOverview }) {
  const t = useTranslations("autopilot.openings");
  const name = useLocaleName();
  const signed = useSigned();
  const gain = useGain(d);
  const predicted = (o: AutopilotOpening) => {
    const g = o.estimate?.gain_monthly;
    if (!g || o.estimate?.confidence === "insufficient") return t("predictedNone");
    const r = o.estimate?.revenue_monthly ?? null;
    const shown = gain(g, r);
    return t("predicted", { gain: shown.main, range: shown.range });
  };
  return (
    <Section title={t("title")}>
      <ul className="grid gap-3 md:grid-cols-2">
        {d.openings.map((o) => (
          <li key={o.id} className="rounded-lg border p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="flex items-center gap-2">
                <h3 className="font-medium">{name(o.locale)}</h3>
                <span className="font-mono text-xs text-muted-foreground">{o.locale}</span>
              </div>
              {o.launch ? <VerdictBadge verdict={o.launch.verdict} /> : <Badge variant={o.status === "failed" ? "destructive" : "outline"}>{t(`status.${o.status}`)}</Badge>}
            </div>
            {o.status === "translating" && o.job_total != null && o.job_total > 0 && (
              <p className="mt-2 text-sm tabular-nums text-muted-foreground">{t("progress", { done: o.job_done ?? 0, total: o.job_total })}</p>
            )}
            {o.error && <p className="mt-2 text-sm text-destructive">{o.error}</p>}
            <p className="mt-2 text-sm text-muted-foreground">{predicted(o)}</p>
            {o.launch && (
              <>
                <p className="mt-1 text-sm tabular-nums">
                  {t("measured", { lift: signed(o.launch.lift), low: signed(o.launch.lift_lo), high: signed(o.launch.lift_hi) })}
                </p>
                <p className="mt-1 text-xs text-muted-foreground">{t(o.launch.causal ? "causal" : "notCausal")}</p>
                <Link href={`/app/${slug}/insights/launches/${o.launch.id}`} className="mt-2 inline-block text-sm font-medium underline underline-offset-4">
                  {t("details")}
                </Link>
              </>
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Copy({ slug, d }: { slug: string; d: AutopilotOverview }) {
  const t = useTranslations("autopilot.copy");
  const name = useLocaleName();
  const f = useFmt(d);
  const signed = useSigned();
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: qk.autopilot(slug) });
  const start = useMutation({
    mutationFn: (c: { key: string; locale: string }) => api.autopilotOptimize(slug, c),
    onSuccess: (_, c) => {
      refresh();
      toast.success(t("started", { key: c.key }));
    },
    onError: (e) => toast.error(e.message),
  });
  const decide = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "approve" | "stop" }) => api.autopilotDecide(slug, id, action),
    onSuccess: (_, v) => {
      refresh();
      toast.success(t(v.action === "approve" ? "approved" : "stopped"));
    },
    onError: (e) => toast.error(e.message),
  });
  const live = (o: AutopilotOptimization) => o.status === "running" || o.status === "awaiting_approval";
  // A lift is only stated once the test produced one: a winner, or an ended test with its honest interval.
  const stated = (o: AutopilotOptimization) => o.result && (o.status === "awaiting_approval" || o.status === "promoted" || o.status === "no_winner");

  return (
    <Section title={t("title")} subtitle={t("subtitle")}>
      {d.optimizations.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("none")}</p>
      ) : (
        <ul className="grid gap-3">
          {d.optimizations.map((o) => {
            const r = o.result;
            return (
              <li key={o.id} className="rounded-lg border p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate font-mono text-sm" translate="no">
                        {o.key}
                      </span>
                      <span className="text-xs text-muted-foreground">
                        {t(`kind.${o.kind}`)} · {name(o.locale)}
                      </span>
                      <Badge variant={o.status === "promoted" ? "default" : o.status === "rolled_back" ? "destructive" : "outline"}>{t(`status.${o.status}`)}</Badge>
                    </div>
                    {o.reason && <p className="mt-1 text-sm text-muted-foreground">{t(`reason.${o.reason}`, { days: o.max_days })}</p>}
                  </div>
                  {live(o) && (
                    <div className="flex gap-2">
                      {o.status === "awaiting_approval" && (
                        <Button size="sm" onClick={() => decide.mutate({ id: o.id, action: "approve" })} disabled={decide.isPending}>
                          <Check aria-hidden /> {t("approve")}
                        </Button>
                      )}
                      <Button size="sm" variant="outline" onClick={() => decide.mutate({ id: o.id, action: "stop" })} disabled={decide.isPending}>
                        <Square aria-hidden /> {t("stop")}
                      </Button>
                    </div>
                  )}
                </div>
                {r && (
                  <div className="mt-3 text-sm">
                    {stated(o) && (
                      <p>
                        <span className="text-xl font-semibold tabular-nums">{signed(r.lift)}</span>{" "}
                        <span className="text-muted-foreground">
                          {t("lift", { low: signed(r.lift_low), high: signed(r.lift_high) })}
                          {r.revenue_monthly && r.lift_low > 0 && <> · {t("worth", { amount: f.money(r.revenue_monthly.mid) })}</>}
                        </span>
                      </p>
                    )}
                    <p className="mt-1 text-xs text-muted-foreground">{t(o.status === "running" ? "leader" : "winner")}</p>
                    <p className="font-medium">“{r.value}”</p>
                    <p className="mt-1 text-xs tabular-nums text-muted-foreground">
                      {t("sample", { views: r.variant[0], conversions: r.variant[1], cviews: r.control[0], cconversions: r.control[1] })}
                    </p>
                    {stated(o) && r.lift_low > 0 && <p className="mt-1 text-xs text-muted-foreground">{t("caveat")}</p>}
                  </div>
                )}
                {o.dropped.length > 0 && (
                  <ul className="mt-2 space-y-0.5 text-xs text-muted-foreground">
                    {o.dropped.map((x, i) => (
                      <li key={i}>{x.why === "worse" ? t("dropped", { value: x.value }) : t("refused", { why: t(`why.${x.why}`), value: x.value })}</li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <h3 className="mt-6 mb-2 text-sm font-medium">{t("candidates")}</h3>
      {d.candidates.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("noCandidates")}</p>
      ) : (
        <ul className="divide-y rounded-lg border">
          {d.candidates.map((c) => (
            <li key={`${c.key}:${c.locale}`} className="flex flex-wrap items-center justify-between gap-3 px-3 py-2.5">
              <div className="min-w-0">
                <p className="truncate text-sm font-medium">{c.current}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {t(`kind.${c.kind}`)} · {name(c.locale)} · <span className="font-mono" translate="no">{c.key}</span> ·{" "}
                  {c.est_days == null
                    ? c.monthly_visits === 0
                      ? t("noTraffic")
                      : t("noConversions", { goal: d.settings.goal })
                    : c.low_traffic
                      ? t("lowTraffic")
                      : t("estDays", { days: Math.ceil(c.est_days) })}
                </p>
              </div>
              <Button
                size="sm"
                variant="outline"
                onClick={() => start.mutate({ key: c.key, locale: c.locale })}
                disabled={start.isPending || c.est_days == null}
                title={c.est_days == null ? (c.monthly_visits === 0 ? t("noTraffic") : t("noConversions", { goal: d.settings.goal })) : undefined}
              >
                <Sparkles aria-hidden /> {t("optimize")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function SettingsForm({ slug, d }: { slug: string; d: AutopilotOverview }) {
  const t = useTranslations("autopilot");
  const f = useFmt(d);
  const qc = useQueryClient();
  const [s, setS] = useState<AutopilotSettings>(d.settings);
  const save = useMutation({
    mutationFn: () => api.autopilotSettings(slug, s),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.autopilot(slug) });
      toast.success(t("settings.saved"));
    },
    onError: (e) => toast.error(e.message),
  });
  const v = d.value;
  return (
    <Section title={t("settings.title")} subtitle={t("settings.subtitle")}>
      <p className="mb-4 text-sm">
        {!v
          ? t("value.none", { goal: d.settings.goal })
          : t(`value.${v.basis}`, { goal: d.settings.goal, amount: f.money(v.amount, v.currency), events: v.events })}
      </p>
      <form
        className="grid items-end gap-3 sm:grid-cols-[1fr_1fr_6rem_1.4fr_auto]"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="grid gap-1.5">
          <Label htmlFor="ap-goal">{t("settings.goal")}</Label>
          <Input id="ap-goal" value={s.goal} onChange={(e) => setS({ ...s, goal: e.target.value })} className="font-mono" required />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ap-value">{t("settings.value")}</Label>
          <Input
            id="ap-value"
            type="number"
            min={0}
            step="any"
            value={s.value_per_conversion ?? ""}
            placeholder={t("settings.valuePlaceholder")}
            onChange={(e) => setS({ ...s, value_per_conversion: e.target.value ? Number(e.target.value) : null })}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ap-currency">{t("settings.currency")}</Label>
          <Input id="ap-currency" value={s.currency} maxLength={3} onChange={(e) => setS({ ...s, currency: e.target.value.toUpperCase() })} className="font-mono" required />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ap-mode">{t("settings.mode")}</Label>
          <Select value={s.mode} onValueChange={(m) => setS({ ...s, mode: m as AutopilotSettings["mode"] })}>
            <SelectTrigger id="ap-mode" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="approval">{t("settings.approval")}</SelectItem>
              <SelectItem value="auto">{t("settings.auto")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />} {t("settings.save")}
        </Button>
      </form>
    </Section>
  );
}
