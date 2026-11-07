"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleCheck, CircleDashed, Globe, Loader2, Plus, Power, RefreshCw, Search, Trash2 } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, EmptyState, ErrorState, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Link } from "@/i18n/navigation";
import { api, type Project, qk } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { EDGE_MODES, type EdgeDomain, type EdgeMode, type EdgeSettings, edgeApi, edgeKeys } from "./api";

/** Project-wide kill switch, always in the page header: off = every domain serves the origin untouched. */
export function KillSwitch({ slug, settings }: { slug: string; settings: EdgeSettings }) {
  const t = useTranslations("edge.kill");
  const qc = useQueryClient();
  const message = useErrorMessage();
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => edgeApi.putSettings(slug, { ...settings, enabled }),
    onSuccess: (s) => {
      qc.setQueryData(edgeKeys.settings(slug), s);
      toast.success(s.enabled ? t("on") : t("off"));
    },
    onError: (e) => toast.error(message(e)),
  });
  const on = settings.enabled;
  return (
    <label className={cn("flex items-center gap-3 rounded-lg border px-3 py-2 text-sm", !on && "border-warning/50 bg-warning/10")}>
      <Power className={cn("size-4", on ? "text-success-foreground" : "text-warning-foreground")} aria-hidden />
      <span>
        <span className="font-medium">{on ? t("labelOn") : t("labelOff")}</span>
        <span className="block text-xs text-muted-foreground">{t("hint")}</span>
      </span>
      <Switch checked={on} disabled={toggle.isPending} onCheckedChange={(v) => toggle.mutate(v)} aria-label={t("aria")} />
    </label>
  );
}

/** SEO localisation is a Growth (`team`) feature; Pro gets a capped trial (`edge_pages` quota). Self-hosted: no gate. */
function usePlan() {
  const billing = useQuery({ queryKey: qk.billing, queryFn: api.billing });
  const b = billing.data;
  const limits = (b?.limits ?? {}) as Record<string, number | null | undefined>;
  const usage = (b?.usage ?? {}) as Record<string, number | undefined>;
  const limit = limits.edge_pages;
  return {
    allowed: !b || !b.enabled || b.plan === "team" || b.plan === "self_hosted" || (limit ?? 0) > 0 || limit === null,
    used: usage.edge_pages ?? 0,
    limit: b?.enabled ? limit : null,
  };
}

export function Setup({ project }: { project: Project }) {
  const t = useTranslations("edge.setup");
  const format = useFormatter();
  const slug = project.slug;
  const plan = usePlan();
  const domains = useQuery({ queryKey: edgeKeys.domains(slug), queryFn: () => edgeApi.domains(slug) });
  const coverage = useQuery({ queryKey: edgeKeys.coverage(slug), queryFn: () => edgeApi.coverage(slug), retry: false });
  const list = domains.data ?? [];
  const steps = [
    { done: list.length > 0, label: t("steps.domain") },
    { done: list.some((d) => d.verified_at), label: t("steps.dns") },
    { done: list.some((d) => d.verified_at && (d.mode !== "subdomain" || d.tls === "active")), label: t("steps.tls") },
    { done: !!coverage.data?.locales.some((l) => l.ready), label: t("steps.ready") },
  ];

  return (
    <div className="space-y-8">
      <div className="grid gap-4 md:grid-cols-2">
        <div className="rounded-xl border p-5">
          <h2 className="font-medium">{t("dynamic.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("dynamic.body")}</p>
          <Button asChild variant="link" className="mt-2 h-auto p-0">
            <Link href={`/app/${slug}/settings`}>{t("dynamic.cta")}</Link>
          </Button>
        </div>
        <div className="rounded-xl border border-brand/40 p-5">
          <div className="flex items-center gap-2">
            <h2 className="font-medium">{t("seo.title")}</h2>
            <Badge variant="secondary">{t("seo.badge")}</Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">{t("seo.body")}</p>
          {plan.limit !== undefined && (
            <p className="mt-3 text-xs text-muted-foreground tabular-nums">
              {plan.limit === null
                ? t("seo.quotaUnlimited", { used: format.number(plan.used) })
                : t("seo.quota", { used: format.number(plan.used), limit: format.number(plan.limit) })}
            </p>
          )}
        </div>
      </div>

      {!plan.allowed && (
        <div role="note" className="rounded-xl border border-warning/50 bg-warning/10 p-4 text-sm">
          {t("upgrade")}{" "}
          <Link href="/app/settings/billing" className="font-medium underline underline-offset-4">
            {t("upgradeCta")}
          </Link>
        </div>
      )}

      <ol className="grid gap-2 sm:grid-cols-4" aria-label={t("steps.title")}>
        {steps.map((s, i) => (
          <li key={i} className="flex items-center gap-2 rounded-lg border px-3 py-2 text-sm">
            {s.done ? <CircleCheck className="size-4 text-success-foreground" aria-hidden /> : <CircleDashed className="size-4 text-muted-foreground" aria-hidden />}
            <span className={cn(!s.done && "text-muted-foreground")}>{s.label}</span>
            <span className="sr-only">{s.done ? t("steps.done") : t("steps.todo")}</span>
          </li>
        ))}
      </ol>

      <AddDomain project={project} disabled={!plan.allowed} />

      {domains.isPending ? (
        <Skeleton className="h-40 rounded-xl" />
      ) : domains.isError ? (
        <ErrorState error={domains.error} onRetry={() => domains.refetch()} />
      ) : list.length === 0 ? (
        <EmptyState icon={Globe} title={t("empty.title")} body={t("empty.body")} />
      ) : (
        <ul className="space-y-4">
          {list.map((d) => (
            <DomainCard key={d.id} slug={slug} domain={d} source={project.source_locale} />
          ))}
        </ul>
      )}

      {list.some((d) => d.mode === "subdomain" && d.verified_at) && <HreflangSnippet slug={slug} />}
    </div>
  );
}

function AddDomain({ project, disabled }: { project: Project; disabled: boolean }) {
  const t = useTranslations("edge.setup");
  const name = useLocaleName();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const [mode, setMode] = useState<EdgeMode>("subdomain");
  const [host, setHost] = useState("");
  const [locale, setLocale] = useState(project.target_locales[0] ?? "");
  const [origin, setOrigin] = useState("");
  const add = useMutation({
    mutationFn: () => edgeApi.addDomain(project.slug, { mode, host: host.trim().toLowerCase(), locale, origin_url: origin.trim() }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: edgeKeys.domains(project.slug) });
      setHost("");
      toast.success(t("added"));
    },
    onError: (e) => toast.error(message(e)),
  });
  return (
    <form
      className="space-y-4 rounded-xl border p-5"
      onSubmit={(e) => {
        e.preventDefault();
        add.mutate();
      }}
    >
      <h2 className="font-medium">{t("add.title")}</h2>
      <fieldset className="grid gap-2 sm:grid-cols-3">
        <legend className="sr-only">{t("add.mode")}</legend>
        {EDGE_MODES.map((m) => (
          <label
            key={m}
            className={cn("cursor-pointer rounded-lg border p-3 text-sm has-checked:border-brand has-checked:bg-brand/5", disabled && "opacity-60")}
          >
            <input type="radio" name="edge-mode" value={m} checked={mode === m} onChange={() => setMode(m)} className="sr-only" disabled={disabled} />
            <span className="font-medium">{t(`mode.${m}`)}</span>
            <span className="mt-0.5 block text-xs text-muted-foreground">{t(`mode.${m}Hint`)}</span>
          </label>
        ))}
      </fieldset>
      <div className="grid gap-3 sm:grid-cols-[1fr_10rem_1fr_auto] sm:items-end">
        <div className="space-y-1.5">
          <Label htmlFor="edge-host">{mode === "subdomain" ? t("add.host") : t("add.site")}</Label>
          <Input
            id="edge-host"
            required
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder={mode === "subdomain" ? "es.example.com" : "example.com"}
            dir="ltr"
            translate="no"
            className="font-mono"
            disabled={disabled}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="edge-locale">{t("add.locale")}</Label>
          <Select value={locale} onValueChange={setLocale} disabled={disabled}>
            <SelectTrigger id="edge-locale" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {project.target_locales.map((l) => (
                <SelectItem key={l} value={l}>
                  {name(l)} <span className="font-mono text-muted-foreground">{l}</span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="edge-origin">{t("add.origin")}</Label>
          <Input
            id="edge-origin"
            required
            type="url"
            value={origin}
            onChange={(e) => setOrigin(e.target.value)}
            placeholder="https://example.com"
            dir="ltr"
            translate="no"
            className="font-mono"
            disabled={disabled}
          />
        </div>
        <Button type="submit" disabled={disabled || add.isPending || !locale}>
          {add.isPending ? <Loader2 className="animate-spin" /> : <Plus />}
          {t("add.submit")}
        </Button>
      </div>
    </form>
  );
}

/** Copyable DNS records + per-mode wiring snippet, verification and per-domain kill switch. */
function DomainCard({ slug, domain: d, source }: { slug: string; domain: EdgeDomain; source: string }) {
  const t = useTranslations("edge.setup");
  const format = useFormatter();
  const qc = useQueryClient();
  const message = useErrorMessage();
  const origin = useOrigin() ?? "";
  const refresh = () => qc.invalidateQueries({ queryKey: edgeKeys.domains(slug) });
  const verify = useMutation({
    mutationFn: () => edgeApi.verifyDomain(slug, d.id),
    onSuccess: (r) => {
      refresh();
      if (r.verified_at) toast.success(t("verified"));
      else toast.error(t("notYet"));
    },
    onError: (e) => toast.error(message(e)),
  });
  const toggle = useMutation({ mutationFn: (enabled: boolean) => edgeApi.updateDomain(slug, d.id, { enabled }), onSuccess: refresh });
  const remove = useMutation({ mutationFn: () => edgeApi.deleteDomain(slug, d.id), onSuccess: refresh });

  const records = [
    ...(d.mode === "subdomain" ? [`${d.host}.  CNAME  ${d.cname_target ?? "edge.glotlang.com"}.`] : []),
    `_glot.${d.host}.  TXT  "${d.verify_token}"`,
  ].join("\n");
  const wiring =
    d.mode === "subdirectory"
      ? `# wrangler.toml (Cloudflare Worker on ${d.host}/${d.locale}/*)\nmain = "node_modules/@glot-i18n/edge/dist/worker.js"\n[vars]\nGLOT_SERVER = "${origin}"\nGLOT_PROJECT = "${slug}"\nLOCALES = "${d.locale}"\nSOURCE_LOCALE = "${source}"`
      : d.mode === "middleware"
        ? `// proxy.ts (Next.js) — npm i @glot-i18n/edge\nimport { glotProxy } from "@glot-i18n/edge/next";\nexport const proxy = glotProxy({ source: "${source}", locales: ["${d.locale}"], edge: { server: "${origin}", project: "${slug}" } });`
        : null;
  const check = (ok: boolean | undefined, label: string) => (
    <Badge variant="secondary" className={cn("gap-1", ok ? "bg-success/12 text-success-foreground" : "text-muted-foreground")}>
      {ok ? <CircleCheck className="size-3" aria-hidden /> : <CircleDashed className="size-3" aria-hidden />}
      {label}
    </Badge>
  );

  return (
    <li className={cn("space-y-4 rounded-xl border p-5", !d.enabled && "opacity-80")}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <span className="font-mono text-sm font-medium" translate="no" dir="ltr">
          {d.mode === "subdomain" ? d.host : `${d.host}/${d.locale}/`}
        </span>
        <Badge variant="outline">{t(`mode.${d.mode}`)}</Badge>
        {check(!!d.verified_at, d.verified_at ? t("status.verified") : t("status.unverified"))}
        {d.mode === "subdomain" && check(d.tls === "active", t(`status.tls.${d.tls ?? "pending"}`))}
        <div className="ms-auto flex items-center gap-2">
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <Switch size="sm" checked={d.enabled} onCheckedChange={(v) => toggle.mutate(v)} disabled={toggle.isPending} />
            {d.enabled ? t("domainOn") : t("domainOff")}
          </label>
          <Button size="sm" variant="outline" onClick={() => verify.mutate()} disabled={verify.isPending}>
            {verify.isPending ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            {t("verify")}
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("remove")}
            onClick={() => window.confirm(t("removeConfirm", { host: d.host })) && remove.mutate()}
          >
            <Trash2 />
          </Button>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("originLine", { origin: d.origin_url })}
        {d.checked_at && <> · {t("checkedAt", { when: format.dateTime(new Date(d.checked_at), { dateStyle: "medium", timeStyle: "short" }) })}</>}
      </p>
      {!d.verified_at && (
        <>
          <p className="text-sm">{d.mode === "subdomain" ? t("dnsSubdomain") : t("dnsTxt")}</p>
          <CodeBlock label={t("dnsLabel")} code={records} />
          {d.checks && (
            <ul className="space-y-1 text-xs text-muted-foreground" aria-live="polite">
              {(["cname", "txt"] as const).map((k) => {
                const c = d.checks?.[k];
                return (
                  c && (
                    <li key={k}>
                      {check(c.ok, k.toUpperCase())}{" "}
                      <span dir="ltr" className="font-mono">
                        {c.found.length ? c.found.join(", ") : t("dnsNothing")}
                      </span>
                    </li>
                  )
                );
              })}
            </ul>
          )}
        </>
      )}
      {wiring && <CodeBlock label={t(`wiring.${d.mode}`)} code={wiring} />}
    </li>
  );
}

/** Subdomain mode: the source site is not behind glot, so it must carry the hreflang tags itself (reciprocity). */
function HreflangSnippet({ slug }: { slug: string }) {
  const t = useTranslations("edge.setup.hreflang");
  const [path, setPath] = useState("/");
  const [asked, setAsked] = useState("/");
  const q = useQuery({ queryKey: [...edgeKeys.all(slug), "hreflang", asked], queryFn: () => edgeApi.hreflang(slug, asked), retry: false });
  return (
    <section className="space-y-3 rounded-xl border p-5">
      <div>
        <h2 className="font-medium">{t("title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("body")}</p>
      </div>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          setAsked(path.startsWith("/") ? path : `/${path}`);
        }}
      >
        <Input aria-label={t("path")} value={path} onChange={(e) => setPath(e.target.value)} dir="ltr" translate="no" className="max-w-64 font-mono" />
        <Button type="submit" variant="outline">
          <Search />
          {t("show")}
        </Button>
      </form>
      {q.isError ? <ErrorState error={q.error} /> : q.data && <CodeBlock code={q.data.tags.join("\n")} />}
    </section>
  );
}
