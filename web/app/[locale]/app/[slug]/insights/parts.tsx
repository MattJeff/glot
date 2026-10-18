"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, CircleDashed, FlaskConical, Loader2, Minus, RefreshCw, ShieldCheck, X } from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, CopyButton, useLocaleName } from "@/components/common";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, qk, type InsightDemand, type InsightExperiment, type InsightFunnel, type InsightVerdict, type Insights } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { cn } from "@/lib/utils";

// Shell command shown as a code label, not UI copy.
const NPM_INSTALL = "npm i @glot-i18n/insights";

export function usePct() {
  const format = useFormatter();
  return (n: number, digits = 1) => format.number(n, { style: "percent", maximumFractionDigits: digits });
}

export function Section({
  title,
  subtitle,
  actions,
  className,
  children,
}: {
  title: string;
  subtitle?: string;
  actions?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <section className={cn("rounded-xl border p-4 sm:p-5", className)}>
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="font-medium">{title}</h2>
          {subtitle && <p className="mt-0.5 text-sm text-muted-foreground">{subtitle}</p>}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}

// ---------- demand gap ----------

export function DemandTable({
  demand,
  running,
  onTest,
}: {
  demand: InsightDemand[];
  running: string[];
  onTest: (d: InsightDemand) => void;
}) {
  const t = useTranslations("insights.demand");
  const format = useFormatter();
  const name = useLocaleName();
  const pct = usePct();
  const max = Math.max(...demand.map((d) => d.share), 0.0001);
  const usd = (n: number) => format.number(n, { style: "currency", currency: "USD", maximumFractionDigits: n < 10 ? 2 : 0 });
  const testing = (l: string) => running.some((r) => r.split("-")[0] === l);

  return (
    <div className="-mx-4 overflow-x-auto sm:-mx-5">
      <table className="w-full min-w-[760px] text-sm">
        <thead className="border-y bg-muted/40 text-xs text-muted-foreground">
          <tr>
            <th scope="col" className="px-4 py-2 text-start font-medium sm:ps-5">{t("language")}</th>
            <th scope="col" className="w-[30%] px-4 py-2 text-start font-medium">{t("share")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium">{t("conversion")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium">{t("potential")}</th>
            <th scope="col" className="px-4 py-2 text-end font-medium">{t("cost")}</th>
            <th scope="col" className="px-4 py-2 sm:pe-5"><span className="sr-only">{t("action")}</span></th>
          </tr>
        </thead>
        <tbody className="divide-y">
          {demand.map((d) => (
            <tr key={d.language} className={cn("transition-colors hover:bg-muted/30", d.offered && "text-muted-foreground")}>
              <td className="px-4 py-2.5 sm:ps-5">
                <div className="flex items-center gap-2">
                  <span className={cn("font-medium", !d.offered && "text-foreground")}>{name(d.language)}</span>
                  <span className="font-mono text-xs text-muted-foreground">{d.language}</span>
                </div>
                {d.top_countries.length > 0 && (
                  <div className="mt-0.5 text-xs text-muted-foreground">{d.top_countries.map(([c]) => c).join(" · ")}</div>
                )}
              </td>
              <td className="px-4 py-2.5">
                <div className="flex items-center gap-3" title={t("visitsTitle", { n: d.visits })}>
                  <div className="h-2.5 flex-1">
                    <div
                      className={cn("h-full rounded-e-[4px]", d.offered ? "bg-muted-foreground/30" : "bg-info")}
                      style={{ width: `${Math.max(1.5, (d.share / max) * 100)}%` }}
                    />
                  </div>
                  <span className="w-12 text-end tabular-nums text-foreground">{pct(d.share)}</span>
                </div>
              </td>
              <td className="px-4 py-2.5 text-end tabular-nums">
                {d.conversion_rate != null ? pct(d.conversion_rate) : "—"}
              </td>
              <td className="px-4 py-2.5 text-end tabular-nums">
                {d.est_extra_conversions != null ? (
                  <span className="text-foreground">+{format.number(d.est_extra_conversions, { maximumFractionDigits: 0 })}</span>
                ) : (
                  "—"
                )}
              </td>
              <td className="px-4 py-2.5 text-end tabular-nums">{d.est_cost_usd != null ? usd(d.est_cost_usd) : "—"}</td>
              <td className="px-4 py-2.5 text-end sm:pe-5">
                {d.offered ? (
                  <Badge variant="secondary" className="gap-1 font-normal">
                    <Check className="size-3" /> {t("offered")}
                  </Badge>
                ) : testing(d.language) ? (
                  <Badge variant="secondary" className="gap-1 bg-info/12 font-normal text-info-foreground">
                    <FlaskConical className="size-3" /> {t("testing")}
                  </Badge>
                ) : (
                  <Button size="xs" variant="outline" onClick={() => onTest(d)}>
                    <FlaskConical /> {t("test")}
                  </Button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ---------- daily ----------

export function DailyChart({ daily }: { daily: Insights["daily"] }) {
  const t = useTranslations("insights.daily");
  const format = useFormatter();
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(...daily.map((d) => d.visits), 1);
  const point = hover != null ? daily[hover] : daily.at(-1);
  const day = (s: string) => format.dateTime(new Date(`${s}T00:00:00Z`), { day: "numeric", month: "short", timeZone: "UTC" });

  if (daily.length === 0) return <p className="text-sm text-muted-foreground">—</p>;
  return (
    <div>
      <div className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground" aria-live="polite">
        <span className="font-medium text-foreground">{point && day(point.day)}</span>
        <span className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-sm bg-info" /> {t("unserved")} · <b className="font-medium text-foreground tabular-nums">{format.number(point?.unserved ?? 0)}</b>
        </span>
        <span className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-sm bg-muted-foreground/30" /> {t("served")} · <b className="font-medium text-foreground tabular-nums">{format.number((point?.visits ?? 0) - (point?.unserved ?? 0))}</b>
        </span>
      </div>
      <div className="relative">
        <div className="pointer-events-none absolute inset-x-0 top-0 border-t border-border/70" />
        <span className="absolute -top-2 end-0 bg-background ps-1 text-[11px] text-muted-foreground tabular-nums">{format.number(max)}</span>
        <div className="flex h-40 items-end gap-[2px] border-b pt-3" onMouseLeave={() => setHover(null)}>
          {daily.map((d, i) => (
            <div
              key={d.day}
              className="flex h-full min-w-0 flex-1 cursor-default flex-col items-center justify-end"
              onMouseEnter={() => setHover(i)}
              role="img"
              aria-label={`${day(d.day)}: ${d.visits} (${d.unserved} ${t("unserved")})`}
            >
              <div
                className={cn("flex w-full max-w-6 flex-col gap-[2px] transition-opacity", hover != null && hover !== i && "opacity-50")}
                style={{ height: `${(d.visits / max) * 100}%` }}
              >
                <div className="w-full flex-1 rounded-t-[4px] bg-muted-foreground/30" />
                <div className="w-full bg-info" style={{ height: `${(d.unserved / Math.max(1, d.visits)) * 100}%` }} />
              </div>
            </div>
          ))}
        </div>
        <div className="mt-1.5 flex justify-between text-[11px] text-muted-foreground tabular-nums">
          <span>{day(daily[0].day)}</span>
          <span>{day(daily.at(-1)!.day)}</span>
        </div>
      </div>
    </div>
  );
}

// ---------- funnel (rate + 95 % CI per served locale) ----------

export function FunnelPlot({ funnel }: { funnel: InsightFunnel[] }) {
  const t = useTranslations("insights.funnel");
  const format = useFormatter();
  const name = useLocaleName();
  const pct = usePct();
  const rows = [...funnel].sort((a, b) => Number(b.is_source) - Number(a.is_source) || b.visits - a.visits);
  const max = Math.max(...rows.map((r) => r.ci_high), 0.01) * 1.1;
  const source = rows.find((r) => r.is_source);
  const x = (v: number) => `${(v / max) * 100}%`;

  if (rows.length === 0) return <p className="text-sm text-muted-foreground">{t("empty")}</p>;
  return (
    <div>
      <ul className="space-y-3">
        {rows.map((r) => {
          const delta = source && !r.is_source ? r.rate - source.rate : null;
          return (
            <li
              key={r.locale}
              className="grid grid-cols-[minmax(0,7rem)_1fr_auto] items-center gap-3"
              title={t("ciTitle", { low: pct(r.ci_low), high: pct(r.ci_high) })}
            >
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">{name(r.locale)}</div>
                <div className="text-[11px] text-muted-foreground">
                  {r.is_source ? t("source") : <span className="font-mono">{r.locale}</span>} {t("sampleSize", { n: format.number(r.visits) })}
                </div>
              </div>
              <div className="relative h-5">
                <div className="absolute inset-x-0 top-1/2 h-px bg-border" />
                {source && <div className="absolute inset-y-0 w-px bg-foreground/40" style={{ insetInlineStart: x(source.rate) }} />}
                <div
                  className="absolute top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-info/45"
                  style={{ insetInlineStart: x(r.ci_low), width: `calc(${x(r.ci_high)} - ${x(r.ci_low)})` }}
                />
                <div
                  className="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-info ring-2 ring-background rtl:translate-x-1/2"
                  style={{ insetInlineStart: x(r.rate) }}
                />
              </div>
              <div className="w-20 text-end">
                <div className="text-sm font-medium tabular-nums">{pct(r.rate)}</div>
                {delta != null && (
                  <div className="text-[11px] tabular-nums text-muted-foreground">
                    {delta >= 0 ? "+" : "−"}
                    {format.number(Math.abs(delta) * 100, { maximumFractionDigits: 1 })} {t("pts")}
                  </div>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      <p className="mt-4 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="inline-block h-3 w-px bg-foreground/40" /> {t("legendSource")}
        <span className="ms-2 inline-block h-0.5 w-4 rounded-full bg-info/45" /> {t("legendCi")}
      </p>
    </div>
  );
}

// ---------- experiments ----------

const VERDICT: Record<InsightVerdict, { icon: React.ComponentType<{ className?: string }>; className: string }> = {
  collecting: { icon: CircleDashed, className: "bg-muted text-muted-foreground" },
  keep: { icon: Check, className: "bg-success/12 text-success-foreground" },
  remove: { icon: X, className: "bg-destructive/10 text-destructive" },
  inconclusive: { icon: Minus, className: "bg-warning/15 text-warning-foreground" },
};

export function ExperimentCard({
  exp,
  sourceLocale,
  busy,
  onDecide,
}: {
  exp: InsightExperiment;
  sourceLocale: string;
  busy: boolean;
  onDecide: (status: "kept" | "removed") => void;
}) {
  const t = useTranslations("insights.exp");
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  const name = useLocaleName();
  const pct = usePct();
  const v = VERDICT[exp.verdict];
  const pts = (n: number) => `${n >= 0 ? "+" : "−"}${format.number(Math.abs(n) * 100, { maximumFractionDigits: 1 })}`;
  const span = Math.max(Math.abs(exp.ci_low), Math.abs(exp.ci_high), 0.001);
  const pos = (n: number) => `${50 + (Math.max(-span, Math.min(span, n)) / span) * 50}%`;
  const sample = Math.min(exp.test.visits, exp.control.visits);
  const running = exp.status === "running";
  const decided = exp.verdict !== "collecting";

  return (
    <div className="flex flex-col rounded-lg border p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="truncate font-medium">{name(exp.locale)}</h3>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">{exp.locale}</span>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("meta", { pct: exp.traffic_pct, goal: exp.goal })} ·{" "}
            {running
              ? new Date(exp.ends_at) > now
                ? t("endsIn", { when: format.relativeTime(new Date(exp.ends_at), now) })
                : t("ended")
              : t(exp.status === "kept" ? "kept" : "removed")}
          </p>
        </div>
        <Badge variant="secondary" className={cn("gap-1 border-0 font-medium", v.className)}>
          <v.icon className="size-3" /> {t(`verdict.${exp.verdict}`)}
        </Badge>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
        {(["test", "control"] as const).map((arm) => (
          <div key={arm} className="rounded-md bg-muted/40 p-3">
            <div className="text-xs text-muted-foreground">
              {arm === "test" ? t("armTest", { language: name(exp.locale) }) : t("armControl", { language: name(sourceLocale) })}
            </div>
            <div className="mt-1 text-lg font-semibold tabular-nums">{pct(exp[arm].rate)}</div>
            <div className="text-xs text-muted-foreground tabular-nums">
              {format.number(exp[arm].conversions)} / {format.number(exp[arm].visits)}
            </div>
          </div>
        ))}
      </div>

      <div className="mt-4">
        <div className="flex items-baseline justify-between text-xs">
          <span className="text-muted-foreground">{t("diff")}</span>
          <span className="tabular-nums">
            <b className="font-medium">{pts(exp.diff)} {t("pts")}</b>{" "}
            <span className="text-muted-foreground">[{pts(exp.ci_low)} ; {pts(exp.ci_high)}]</span>
          </span>
        </div>
        <div className="relative mt-2 h-4" role="img" aria-label={t("ciAria", { low: pts(exp.ci_low), high: pts(exp.ci_high) })}>
          <div className="absolute inset-x-0 top-1/2 h-px bg-border" />
          <div className="absolute inset-y-0 start-1/2 w-px bg-foreground/40" />
          {exp.test.visits > 0 && exp.control.visits > 0 && (
            <>
              <div
                className={cn("absolute top-1/2 h-1 -translate-y-1/2 rounded-full", exp.ci_low > 0 ? "bg-success/60" : exp.ci_high < 0 ? "bg-destructive/50" : "bg-info/45")}
                style={{ insetInlineStart: pos(exp.ci_low), width: `calc(${pos(exp.ci_high)} - ${pos(exp.ci_low)})` }}
              />
              <div
                className="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground ring-2 ring-background rtl:translate-x-1/2"
                style={{ insetInlineStart: pos(exp.diff) }}
              />
            </>
          )}
        </div>
      </div>

      <div className="mt-4 text-xs text-muted-foreground">
        {exp.required_per_arm != null ? (
          <>
            <div className="flex justify-between">
              <span>{t("sample")}</span>
              <span className="tabular-nums">
                {format.number(sample)} / {format.number(exp.required_per_arm)}
              </span>
            </div>
            <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-info/15">
              <div className="h-full rounded-full bg-info" style={{ width: `${Math.min(100, (sample / exp.required_per_arm) * 100)}%` }} />
            </div>
          </>
        ) : (
          t("sampleUnknown")
        )}
        <p className="mt-2">{t(`explain.${exp.verdict}`)}</p>
      </div>

      {running && (
        <div className="mt-4 flex gap-2 border-t pt-4">
          <Button size="sm" variant={exp.verdict === "keep" ? "default" : "outline"} disabled={busy} onClick={() => onDecide("kept")}>
            {busy ? <Loader2 className="animate-spin" /> : <Check />} {t("keep")}
          </Button>
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant={exp.verdict === "remove" ? "destructive" : "ghost"} disabled={busy}>
                <X /> {t("remove")}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{t("removeTitle", { language: name(exp.locale) })}</AlertDialogTitle>
                <AlertDialogDescription>{t("removeBody", { language: name(exp.locale) })}</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
                <AlertDialogAction variant="destructive" onClick={() => onDecide("removed")}>
                  {t("remove")}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
          {!decided && <span className="ms-auto self-center text-xs text-muted-foreground">{t("early")}</span>}
        </div>
      )}
    </div>
  );
}

export function ExperimentDialog({
  slug,
  target,
  keys,
  goal,
  onClose,
}: {
  slug: string;
  target: InsightDemand | "new" | null;
  keys: number;
  goal: string;
  onClose: () => void;
}) {
  const t = useTranslations("insights.exp");
  const format = useFormatter();
  const name = useLocaleName();
  const qc = useQueryClient();
  // Remounted per target (key in the page), so the initial state is the reset.
  const [locale, setLocale] = useState(target && target !== "new" ? target.language : "");
  const [traffic, setTraffic] = useState(50);
  const [days, setDays] = useState(14);
  const [g, setG] = useState(goal);
  const create = useMutation({
    mutationFn: () => api.createExperiment(slug, { locale: locale.trim(), traffic_pct: traffic, days, goal: g.trim() || undefined }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.insights(slug) });
      qc.invalidateQueries({ queryKey: qk.project(slug) });
      toast.success(t("started", { language: name(locale.trim()) }));
      onClose();
    },
  });
  const cost = target && target !== "new" ? target.est_cost_usd : null;

  return (
    <Dialog open={!!target} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("dialogTitle")}</DialogTitle>
          <DialogDescription>{t("dialogBody")}</DialogDescription>
        </DialogHeader>
        <form
          id="exp-form"
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="exp-locale">{t("locale")}</Label>
            <Input id="exp-locale" value={locale} onChange={(e) => setLocale(e.target.value)} placeholder="de" translate="no" required className="font-mono" />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="exp-traffic">{t("traffic")}</Label>
              <Input id="exp-traffic" type="number" min={1} max={100} value={traffic} onChange={(e) => setTraffic(Number(e.target.value))} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="exp-days">{t("days")}</Label>
              <Input id="exp-days" type="number" min={1} max={90} value={days} onChange={(e) => setDays(Number(e.target.value))} />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="exp-goal">{t("goal")}</Label>
            <Input id="exp-goal" value={g} onChange={(e) => setG(e.target.value)} className="font-mono" />
          </div>
          <p className="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">
            {cost != null
              ? t("costNote", { cost: format.number(cost, { style: "currency", currency: "USD" }), keys })
              : t("translateNote")}
          </p>
        </form>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button type="submit" form="exp-form" disabled={create.isPending || !locale.trim()}>
            {create.isPending ? <Loader2 className="animate-spin" /> : <FlaskConical />} {t("start")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------- install ----------

export function Install({ slug, publicKey, open }: { slug: string; publicKey: string; open?: boolean }) {
  const t = useTranslations("insights.install");
  const qc = useQueryClient();
  const origin = useOrigin() ?? "https://glot.example.com";
  const rotate = useMutation({
    mutationFn: () => api.rotateInsightsKey(slug),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.insights(slug) });
      toast.success(t("rotated"));
    },
  });
  const script = `<script src="https://unpkg.com/@glot-i18n/insights/dist/insights.global.js"
  data-key="${publicKey}" data-endpoint="${origin}" defer></script>
<!-- conversions -->
<script>glot.track("signup")</script>`;
  const npm = `import { glot } from "@glot-i18n/insights";

glot.init({
  key: "${publicKey}",
  endpoint: "${origin}",
  locale: currentLocale,              // the locale you rendered
  onLocale: (l) => switchLocale(l),   // language experiment: serve l
});

glot.track("signup");                 // or "purchase", any name`;
  const middleware = `// middleware.ts (Next.js, Edge) — first visit only, no client JS
import { reportVisit } from "@glot-i18n/insights/server";
import { NextResponse, type NextRequest } from "next/server";

export async function middleware(req: NextRequest) {
  const res = NextResponse.next();
  if (req.cookies.has("NEXT_LOCALE")) return res; // already counted
  const served = req.nextUrl.pathname.split("/")[1] || "en";
  const r = await reportVisit(req, { key: "${publicKey}", endpoint: "${origin}", served });
  // Test arm of a language experiment: next-intl serves this locale.
  res.cookies.set("NEXT_LOCALE", r?.locale ?? served);
  return res;
}`;

  return (
    <details open={open} className="group rounded-xl border">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 p-4 sm:p-5 [&::-webkit-details-marker]:hidden">
        <div>
          <h2 className="font-medium">{t("title")}</h2>
          <p className="mt-0.5 text-sm text-muted-foreground">{t("subtitle")}</p>
        </div>
        <span className="text-xs text-muted-foreground group-open:hidden">{t("show")}</span>
      </summary>
      <div className="grid gap-5 border-t p-4 sm:p-5 lg:grid-cols-[1fr_16rem]">
        <Tabs defaultValue="script" className="min-w-0">
          <TabsList>
            <TabsTrigger value="script">{t("tabScript")}</TabsTrigger>
            <TabsTrigger value="npm" translate="no">npm</TabsTrigger>
            <TabsTrigger value="middleware">{t("tabMiddleware")}</TabsTrigger>
          </TabsList>
          <TabsContent value="script"><CodeBlock code={script} label="index.html" /></TabsContent>
          <TabsContent value="npm"><CodeBlock code={npm} label={NPM_INSTALL} /></TabsContent>
          <TabsContent value="middleware"><CodeBlock code={middleware} label="middleware.ts" /></TabsContent>
        </Tabs>
        <div className="space-y-4">
          <div>
            <div className="text-xs font-medium text-muted-foreground">{t("publicKey")}</div>
            <div className="mt-1 flex items-center gap-1 rounded-md border bg-muted/40 ps-2.5">
              <code className="min-w-0 flex-1 truncate font-mono text-xs">{publicKey}</code>
              <CopyButton value={publicKey} />
            </div>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="ghost" size="xs" className="mt-1 text-muted-foreground" disabled={rotate.isPending}>
                  <RefreshCw /> {t("rotate")}
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>{t("rotateTitle")}</AlertDialogTitle>
                  <AlertDialogDescription>{t("rotateBody")}</AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
                  <AlertDialogAction onClick={() => rotate.mutate()}>{t("rotate")}</AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
          <ul className="space-y-2 text-xs text-muted-foreground">
            {(["noCookie", "noIp", "daily", "size"] as const).map((k) => (
              <li key={k} className="flex gap-2">
                <ShieldCheck className="mt-px size-3.5 shrink-0 text-success-foreground" /> {t(`privacy.${k}`)}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </details>
  );
}
