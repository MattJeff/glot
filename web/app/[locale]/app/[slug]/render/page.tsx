"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Loader2, Play, Plus, Trash2 } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, CopyButton, EmptyState, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { Link } from "@/i18n/navigation";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";
import { renderApi, renderCurl, renderQk, type RenderResult, type RenderType } from "@/lib/render-protocol";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";

const TYPES: RenderType[] = ["html", "text", "markdown", "template"];

/** Render: paste any content, see it in every language of the project; keys, usage and memory hit rate. */
export default function RenderPage() {
  const t = useTranslations("renderProtocol");
  const slug = useSlug();
  const project = useProject(slug);

  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  return (
    <div className="mx-auto max-w-5xl">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button variant="outline" asChild>
            <Link href="/docs/render">{t("docs")}</Link>
          </Button>
        }
      />
      {project.data ? (
        <Playground slug={slug} locales={project.data.target_locales} source={project.data.source_locale} />
      ) : (
        <Skeleton className="h-72 rounded-xl" />
      )}
      <UsagePanel slug={slug} />
      <Keys slug={slug} />
    </div>
  );
}

function Playground({ slug, locales, source }: { slug: string; locales: string[]; source: string }) {
  const t = useTranslations("renderProtocol");
  const localeName = useLocaleName();
  const errorMessage = useErrorMessage();
  const qc = useQueryClient();
  const [type, setType] = useState<RenderType>("html");
  const [content, setContent] = useState("");
  const [variables, setVariables] = useState("");
  const [picked, setPicked] = useState<string[] | null>(null);
  const selected = picked ?? locales;

  const run = useMutation({
    mutationFn: () => {
      let vars: Record<string, unknown> | undefined;
      if (variables.trim()) {
        try {
          vars = JSON.parse(variables);
        } catch {
          throw new Error(t("play.badJson"));
        }
      }
      return renderApi.run(slug, selected.map((locale) => ({ locale, type, content, variables: vars })));
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: renderQk.usage(slug) }),
    onError: (e) => toast.error(e instanceof Error && !("status" in e) ? e.message : errorMessage(e)),
  });

  return (
    <section aria-labelledby="rp-play">
      <h2 id="rp-play" className="text-base font-semibold">{t("play.title")}</h2>
      <p className="mt-1 text-sm text-muted-foreground">{t("play.hint", { source: localeName(source) })}</p>
      <form
        className="mt-4 space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          run.mutate();
        }}
      >
        <div className="grid gap-3 lg:grid-cols-[1fr_18rem]">
          <div className="space-y-1.5">
            <Label htmlFor="rp-content">{t("play.content")}</Label>
            <Textarea
              id="rp-content"
              dir="auto"
              required
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={t("play.contentPlaceholder")}
              className="min-h-44 font-mono text-sm"
            />
          </div>
          <div className="space-y-3">
            <div className="space-y-1.5">
              <Label htmlFor="rp-type">{t("play.type")}</Label>
              <Select value={type} onValueChange={(v) => setType(v as RenderType)}>
                <SelectTrigger id="rp-type" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TYPES.map((k) => (
                    <SelectItem key={k} value={k}>{t(`types.${k}`)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="rp-vars">{t("play.variables")}</Label>
              <Textarea
                id="rp-vars"
                dir="ltr"
                value={variables}
                onChange={(e) => setVariables(e.target.value)}
                placeholder={'{"first_name": "…", "total": {"type": "currency", "value": 0, "currency": "EUR"}}'}
                className="min-h-20 font-mono text-xs"
              />
              <p className="text-xs text-muted-foreground">{t("play.variablesHint")}</p>
            </div>
          </div>
        </div>
        {locales.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("play.noLocales")}</p>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm text-muted-foreground">{t("play.languages")}</span>
            {locales.map((l) => {
              const on = selected.includes(l);
              return (
                <button
                  key={l}
                  type="button"
                  aria-pressed={on}
                  onClick={() => setPicked(on ? selected.filter((x) => x !== l) : [...selected, l])}
                  className={cn(
                    "rounded-full border px-3 py-1 text-sm transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                    on ? "border-foreground bg-foreground text-background" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {localeName(l)}
                </button>
              );
            })}
            <Button type="submit" className="ms-auto" disabled={run.isPending || selected.length === 0 || !content.trim()}>
              {run.isPending ? <Loader2 className="animate-spin" /> : <Play />}
              {t("play.run")}
            </Button>
          </div>
        )}
      </form>
      {run.data && (
        <div className="mt-5 grid gap-3 md:grid-cols-2">
          {run.data.items.map((r, i) => (
            <Result key={r.locale ?? i} r={r} html={type === "html"} />
          ))}
        </div>
      )}
    </section>
  );
}

function Result({ r, html }: { r: RenderResult; html: boolean }) {
  const t = useTranslations("renderProtocol");
  const localeName = useLocaleName();
  if (r.error) {
    return <p role="alert" className="rounded-xl border border-destructive/40 p-3 text-sm md:col-span-2">{r.error.message}</p>;
  }
  const text = typeof r.content === "string" ? r.content : JSON.stringify(r.content, null, 2);
  const failed = r.status === "fallback" || r.status === "partial";
  return (
    <article className="min-w-0 rounded-xl border">
      <header className="flex flex-wrap items-center gap-2 border-b px-3 py-2">
        <h3 className="text-sm font-medium">{localeName(r.locale)}</h3>
        <Badge variant={failed ? "destructive" : "secondary"}>{t(`status.${r.status}`)}</Badge>
        <span className="ms-auto text-xs text-muted-foreground tabular-nums">
          {t("play.units", { cached: r.units.cached, translated: r.units.translated, total: r.units.total })}
        </span>
        <CopyButton value={text} />
      </header>
      {failed && r.fallback_reason && <p role="status" className="border-b bg-muted/40 px-3 py-2 text-xs">{r.fallback_reason}</p>}
      {html && (
        // Untrusted markup: sandboxed, no scripts, no same-origin.
        <iframe title={t("play.preview", { locale: localeName(r.locale) })} sandbox="" srcDoc={text} className="h-56 w-full border-b bg-white" />
      )}
      <pre dir="auto" className="max-h-56 overflow-auto p-3 text-xs whitespace-pre-wrap">{text}</pre>
    </article>
  );
}

function UsagePanel({ slug }: { slug: string }) {
  const t = useTranslations("renderProtocol");
  const format = useFormatter();
  const usage = useQuery({ queryKey: renderQk.usage(slug), queryFn: () => renderApi.usage(slug), enabled: !!slug });
  return (
    <section aria-labelledby="rp-usage" className="mt-10">
      <h2 id="rp-usage" className="text-base font-semibold">{t("usage.title")}</h2>
      {usage.isPending ? (
        <Skeleton className="mt-4 h-24 rounded-xl" />
      ) : usage.isError ? (
        <ErrorState error={usage.error} onRetry={() => usage.refetch()} />
      ) : usage.data.totals.requests === 0 ? (
        <p className="mt-2 text-sm text-muted-foreground">{t("usage.empty")}</p>
      ) : (
        <>
          <dl className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
            {(
              [
                ["documents", format.number(usage.data.totals.requests)],
                ["hitRate", usage.data.hit_rate === null ? "–" : format.number(usage.data.hit_rate, { style: "percent", maximumFractionDigits: 1 })],
                ["translated", format.number(usage.data.totals.translated)],
                ["fallbacks", format.number(usage.data.totals.fallbacks)],
              ] as const
            ).map(([k, v]) => (
              <div key={k} className="rounded-xl border p-3">
                <dt className="text-xs text-muted-foreground">{t(`usage.${k}`)}</dt>
                <dd className="mt-1 text-xl font-semibold tabular-nums">{v}</dd>
              </div>
            ))}
          </dl>
          <p className="mt-3 text-sm text-muted-foreground">
            {t("usage.memory", { entries: usage.data.memory_entries, locales: usage.data.memory_locales })}
          </p>
          <div className="mt-3 overflow-x-auto rounded-xl border">
            <table className="w-full text-sm">
              <thead className="border-b bg-muted/40 text-xs text-muted-foreground">
                <tr>
                  <th scope="col" className="px-4 py-2 text-start font-medium">{t("usage.day")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium">{t("usage.documents")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium">{t("usage.fromMemory")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium">{t("usage.translated")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium">{t("usage.fallbacks")}</th>
                </tr>
              </thead>
              <tbody className="divide-y tabular-nums">
                {[...usage.data.days].reverse().map((d) => (
                  <tr key={d.day}>
                    <td className="px-4 py-2">{format.dateTime(new Date(`${d.day}T00:00:00Z`), { dateStyle: "medium", timeZone: "UTC" })}</td>
                    <td className="px-4 py-2 text-end">{format.number(d.requests)}</td>
                    <td className="px-4 py-2 text-end">{format.number(d.cached)}</td>
                    <td className="px-4 py-2 text-end">{format.number(d.translated)}</td>
                    <td className="px-4 py-2 text-end">{format.number(d.fallbacks)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function Keys({ slug }: { slug: string }) {
  const t = useTranslations("renderProtocol");
  const tc = useTranslations("common");
  const format = useFormatter();
  const errorMessage = useErrorMessage();
  const origin = useOrigin();
  const qc = useQueryClient();
  const keys = useQuery({ queryKey: renderQk.keys(slug), queryFn: () => renderApi.keys(slug), enabled: !!slug });
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState("render");
  const [secret, setSecret] = useState<string | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: renderQk.keys(slug) });
  const create = useMutation({
    mutationFn: (b: { name: string; scopes: string[]; monthly_limit: number | null }) => renderApi.createKey(slug, b),
    onSuccess: (k) => {
      refresh();
      setOpen(false);
      setSecret(k.key);
    },
    onError: (e) => toast.error(errorMessage(e)),
  });
  const revoke = useMutation({
    mutationFn: (id: string) => renderApi.deleteKey(slug, id),
    onSuccess: () => {
      refresh();
      toast.success(t("keys.revoked"));
    },
    onError: (e) => toast.error(errorMessage(e)),
  });

  return (
    <section aria-labelledby="rp-keys" className="mt-10">
      <div className="flex items-center justify-between gap-3">
        <h2 id="rp-keys" className="text-base font-semibold">{t("keys.title")}</h2>
        <Button variant="outline" onClick={() => setOpen(true)}>
          <Plus /> {t("keys.create")}
        </Button>
      </div>
      <div className="mt-4">
        {keys.isPending ? (
          <Skeleton className="h-24 rounded-xl" />
        ) : keys.isError ? (
          <ErrorState error={keys.error} onRetry={() => keys.refetch()} />
        ) : keys.data.length === 0 ? (
          <EmptyState icon={KeyRound} title={t("keys.emptyTitle")} body={t("keys.emptyBody")} />
        ) : (
          <div className="overflow-x-auto rounded-xl border">
            <table className="w-full text-sm">
              <thead className="border-b bg-muted/40 text-xs text-muted-foreground">
                <tr>
                  <th scope="col" className="px-4 py-2 text-start font-medium">{t("keys.name")}</th>
                  <th scope="col" className="px-4 py-2 text-start font-medium">{t("keys.scope")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium">{t("keys.month")}</th>
                  <th scope="col" className="w-12"><span className="sr-only">{t("keys.revoke")}</span></th>
                </tr>
              </thead>
              <tbody className="divide-y">
                {keys.data.map((k) => (
                  <tr key={k.id}>
                    <td className="px-4 py-2.5">
                      <span className="font-medium">{k.name}</span>{" "}
                      <code dir="ltr" className="text-xs text-muted-foreground">{k.prefix}…</code>
                    </td>
                    <td className="px-4 py-2.5 text-muted-foreground">
                      {t(k.scopes.includes("render") ? "keys.scopeRender" : "keys.scopeCached")}
                    </td>
                    <td className="px-4 py-2.5 text-end tabular-nums">
                      {k.monthly_limit === null
                        ? format.number(k.used_this_month)
                        : t("keys.of", { used: k.used_this_month, limit: k.monthly_limit })}
                    </td>
                    <td className="px-2">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={t("keys.revokeNamed", { name: k.name })}
                        disabled={revoke.isPending}
                        onClick={() => revoke.mutate(k.id)}
                      >
                        <Trash2 />
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <div className="mt-4">
        <CodeBlock label="curl" code={renderCurl(origin)} />
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-md">
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              const limit = String(f.get("limit") ?? "").trim();
              create.mutate({ name: String(f.get("name")).trim(), scopes: [scope], monthly_limit: limit ? Number(limit) : null });
            }}
          >
            <DialogHeader>
              <DialogTitle>{t("keys.create")}</DialogTitle>
              <DialogDescription>{t("keys.createHint")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-1.5">
              <Label htmlFor="rk-name">{t("keys.name")}</Label>
              <Input id="rk-name" name="name" required maxLength={100} />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="rk-scope">{t("keys.scope")}</Label>
              <Select value={scope} onValueChange={setScope}>
                <SelectTrigger id="rk-scope" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="render">{t("keys.scopeRender")}</SelectItem>
                  <SelectItem value="render:cached">{t("keys.scopeCached")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="rk-limit">{t("keys.limit")}</Label>
              <Input id="rk-limit" name="limit" type="number" min={0} step={1} inputMode="numeric" />
              <p className="text-xs text-muted-foreground">{t("keys.limitHint")}</p>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setOpen(false)}>{tc("cancel")}</Button>
              <Button type="submit" disabled={create.isPending}>
                {create.isPending && <Loader2 className="animate-spin" />}
                {tc("create")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!secret} onOpenChange={(o) => !o && setSecret(null)}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("keys.revealTitle")}</DialogTitle>
            <DialogDescription>{t("keys.revealBody")}</DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2 rounded-lg border bg-muted/40 p-2 ps-3">
            <code dir="ltr" className="flex-1 font-mono text-sm break-all">{secret}</code>
            <CopyButton value={secret ?? ""} />
          </div>
          <CodeBlock label="curl" code={renderCurl(origin, secret ?? undefined)} />
          <DialogFooter>
            <Button onClick={() => setSecret(null)}>{tc("close")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
