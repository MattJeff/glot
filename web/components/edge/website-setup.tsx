"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CircleCheck, CircleX, Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { CodeBlock, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link } from "@/i18n/navigation";
import { api, type Project, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { type EdgeSettings, edgeApi, edgeKeys, type WebsiteCrawl, type WebsiteScan } from "./api";
import { WebsiteEditor } from "./website-editor";

type Guide = { id: string; name: string; injection: { steps: string[]; plans: string; available: boolean } };
// ponytail: a fixed shortlist by reach, minus what the project already has; real visitor languages (Insights)
// are the upgrade once the tag has traffic.
const POPULAR = ["en", "es", "fr", "de", "pt", "it", "ja", "zh", "ar", "nl", "ko"];

function Step({ n, title, done, children }: { n: number; title: string; done?: boolean; children: ReactNode }) {
  return (
    <li className="space-y-4 rounded-xl border p-5">
      <h3 className="flex items-center gap-3 font-medium">
        <span aria-hidden className="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary text-xs text-primary-foreground">
          {n}
        </span>
        {title}
        {done && <CircleCheck aria-hidden className="size-4 text-emerald-600" />}
      </h3>
      {children}
    </li>
  );
}
const Note = ({ children }: { children: ReactNode }) => (
  <p className="flex gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
    <AlertTriangle aria-hidden className="mt-0.5 size-4 shrink-0 text-amber-600" />
    <span>{children}</span>
  </p>
);

/**
 * The website tag for someone with no developer: enter the address → glot reads the public pages, translates them
 * and shows the result (nothing installed yet) → paste one line where the detected platform wants it → glot checks
 * the live site and says exactly what is wrong when it is not there.
 */
export function WebsiteSetup({ project, settings, tag }: { project: Project; settings: EdgeSettings; tag: string }) {
  const t = useTranslations("edge.website.setup");
  const slug = project.slug;
  const qc = useQueryClient();
  const message = useErrorMessage();
  const localeName = useLocaleName();
  const w = settings.website_snippet;
  const [url, setUrl] = useState(w.site);
  const [scan, setScan] = useState<WebsiteScan | null>(null);
  const [crawl, setCrawl] = useState<WebsiteCrawl | null>(null);
  const [checked, setChecked] = useState<WebsiteScan | null>(null);
  const [excluded, setExcluded] = useState<string[] | null>(null);

  const analyze = useMutation({
    mutationFn: async () => {
      const s = await edgeApi.websiteScan(slug, url);
      // Reading and translating the pages needs a reachable site; the scan says why when it is not.
      const c = s.error ? null : await edgeApi.websiteCrawl(slug, s.url);
      return { s, c };
    },
    onSuccess: ({ s, c }) => {
      setScan(s);
      setCrawl(c);
      setExcluded(null);
      void qc.invalidateQueries({ queryKey: edgeKeys.settings(slug) });
    },
    onError: (e) => toast.error(message(e)),
  });
  const verify = useMutation({
    mutationFn: () => edgeApi.websiteScan(slug, url || w.site),
    onSuccess: setChecked,
    onError: (e) => toast.error(message(e)),
  });
  const put = useMutation({
    mutationFn: (patch: (cur: EdgeSettings) => EdgeSettings) => edgeApi.putSettings(slug, patch(qc.getQueryData<EdgeSettings>(edgeKeys.settings(slug)) ?? settings)),
    onSuccess: (r) => qc.setQueryData(edgeKeys.settings(slug), r),
    onError: (e) => toast.error(message(e)),
  });
  const addLocale = useMutation({
    mutationFn: (l: string) => api.updateProject(slug, { target_locales: [...project.target_locales, l] }),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.project(slug) }),
    onError: (e) => toast.error(message(e)),
  });
  // Platform guides (content/integrations/platforms.json, 130 kB): fetched only once a platform is detected.
  const platformId = (checked ?? scan)?.platform ?? null;
  const guide = useQuery({
    queryKey: ["integration-guide", platformId],
    queryFn: async () => ((await import("@/content/integrations/platforms.json")).default.platforms as Guide[]).find((p) => p.id === platformId) ?? null,
    enabled: !!platformId,
    staleTime: Infinity,
  });

  const site = w.site || (crawl && !crawl.error && scan ? new URL(scan.url).origin : "");
  const suggestions = (scan?.suggested_exclude ?? []).filter((p) => !settings.discover_exclude.includes(p));
  const toExclude = excluded ?? suggestions;
  const more = POPULAR.filter((l) => l !== project.source_locale.split("-")[0] && !project.target_locales.some((x) => x.split("-")[0] === l)).slice(0, 4);
  const hosts = typeof window === "undefined" ? "" : [location.host, location.host.replace(/^app\./, "api.")].filter((h, i, a) => a.indexOf(h) === i).join(", ");
  const tagOk = !!checked?.tag.found && checked.tag.right_project;
  const error = (e: WebsiteScan["error"]) => (e ? t(`errors.${e}`) : null);

  return (
    <section className="space-y-4" aria-labelledby="website-setup-title">
      <div>
        <h2 id="website-setup-title" className="font-medium">
          {t("title")}
        </h2>
        <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{t("intro")}</p>
      </div>
      <ol className="space-y-4">
        <Step n={1} title={t("step1")} done={!!crawl && !crawl.error}>
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              analyze.mutate();
            }}
          >
            <div className="grow space-y-1.5">
              <Label htmlFor="website-setup-url">{t("urlLabel")}</Label>
              <Input id="website-setup-url" required inputMode="url" autoComplete="url" placeholder="www.example.com" value={url} onChange={(e) => setUrl(e.target.value)} dir="ltr" translate="no" />
            </div>
            <Button type="submit" disabled={analyze.isPending}>
              {analyze.isPending && <Loader2 className="animate-spin" />}
              {analyze.isPending ? t("analyzing") : t("analyze")}
            </Button>
          </form>
          <div aria-live="polite" className="space-y-3">
            {scan && (scan.error || crawl?.error) && (
              <p className="flex gap-2 text-sm text-destructive">
                <CircleX aria-hidden className="mt-0.5 size-4 shrink-0" />
                {error(scan.error ?? crawl?.error ?? null)}
              </p>
            )}
            {scan && crawl && !scan.error && !crawl.error && (
              <>
                <ul className="space-y-1 text-sm">
                  <li>{guide.data ? t("detected", { platform: guide.data.name }) : t("unknownPlatform")}</li>
                  {scan.lang && <li>{t("language", { language: localeName(scan.lang) })}</li>}
                  <li>{t("crawled", { pages: crawl.pages, segments: crawl.segments })}</li>
                  <li>{crawl.queued.length ? t("translating", { languages: crawl.queued.map((l) => localeName(l)).join(", ") }) : t("upToDate")}</li>
                </ul>
                {scan.other_tools.length > 0 && <Note>{t("warn.otherTools", { tools: scan.other_tools.join(", ") })}</Note>}
                {scan.js_only && <Note>{t("warn.jsOnly")}</Note>}
                {more.length > 0 && (
                  <div className="space-y-2">
                    <p className="text-sm text-muted-foreground">{t("suggest")}</p>
                    <div className="flex flex-wrap gap-2">
                      {more.map((l) => (
                        <Button key={l} type="button" variant="outline" size="sm" disabled={addLocale.isPending} onClick={() => addLocale.mutate(l)}>
                          {t("add", { language: localeName(l) })}
                        </Button>
                      ))}
                    </div>
                  </div>
                )}
                <div className="space-y-2">
                  <p className="text-sm font-medium">{t("exclude.title")}</p>
                  <p className="max-w-3xl text-sm text-muted-foreground">{suggestions.length ? t("exclude.body") : t("exclude.none")}</p>
                  {suggestions.length > 0 && (
                    <>
                      <div className="flex flex-wrap gap-4">
                        {suggestions.map((p) => (
                          <Label key={p} className="flex items-center gap-2 font-normal">
                            <Checkbox checked={toExclude.includes(p)} onCheckedChange={(c) => setExcluded(c ? [...toExclude, p] : toExclude.filter((x) => x !== p))} />
                            <code dir="ltr" translate="no" className="font-mono text-xs">
                              {p}
                            </code>
                          </Label>
                        ))}
                      </div>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={put.isPending || !toExclude.length}
                        onClick={() => put.mutate((cur) => ({ ...cur, discover_exclude: [...new Set([...cur.discover_exclude, ...toExclude])] }), { onSuccess: () => toast.success(t("exclude.done")) })}
                      >
                        {t("exclude.apply")}
                      </Button>
                    </>
                  )}
                </div>
              </>
            )}
          </div>
        </Step>

        <Step n={2} title={t("step2")}>
          {site ? <WebsiteEditor project={project} settings={settings} site={site} refresh={!!crawl?.queued.length} /> : <p className="text-sm text-muted-foreground">{t("previewLater")}</p>}
        </Step>

        <Step n={3} title={t("step3")} done={tagOk && w.on}>
          <CodeBlock label={t("install.copy")} code={tag} />
          {guide.data ? (
            <div className="space-y-2">
              <p className="text-sm font-medium">{t("install.where", { platform: guide.data.name })}</p>
              {/* Each platform's own menu names, in English on purpose (content/integrations/platforms.json). */}
              <ol className="list-decimal space-y-1 ps-5 text-sm" lang="en">
                {guide.data.injection.steps.map((s) => (
                  <li key={s}>{s}</li>
                ))}
              </ol>
              <p className="text-xs text-muted-foreground" lang="en">
                {guide.data.injection.plans}
              </p>
              <Button variant="link" className="h-auto p-0" asChild>
                <Link href={`/integrations/${guide.data.id}`} target="_blank">
                  {t("install.guide", { platform: guide.data.name })}
                </Link>
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              {t("install.generic")}{" "}
              <Link href="/integrations" target="_blank" className="underline underline-offset-4">
                {t("install.allGuides")}
              </Link>
            </p>
          )}
          <div className="flex flex-wrap items-center gap-3">
            <Button type="button" variant="outline" disabled={verify.isPending || !(url || w.site)} onClick={() => verify.mutate()}>
              {verify.isPending && <Loader2 className="animate-spin" />}
              {t("verify.button")}
            </Button>
            {tagOk && !w.on && (
              <Button
                type="button"
                disabled={put.isPending}
                onClick={async () => {
                  // Visitors' browsers read the translations: they have to be public before the tag goes live.
                  if (!project.public_cdn) await api.updateProject(slug, { public_cdn: true }).then(() => qc.invalidateQueries({ queryKey: qk.project(slug) }));
                  put.mutate((cur) => ({ ...cur, website_snippet: { ...cur.website_snippet, on: true } }));
                }}
              >
                {t("verify.turnOn")}
              </Button>
            )}
          </div>
          <div aria-live="polite" className="space-y-2 text-sm">
            {checked?.error && (
              <p className="flex gap-2 text-destructive">
                <CircleX aria-hidden className="mt-0.5 size-4 shrink-0" />
                {error(checked.error)}
              </p>
            )}
            {checked && !checked.error && !checked.tag.found && (
              <p className="flex gap-2 text-destructive">
                <CircleX aria-hidden className="mt-0.5 size-4 shrink-0" />
                {t("verify.notFound", { url: checked.url })}
              </p>
            )}
            {checked?.tag.found && !checked.tag.right_project && (
              <p className="flex gap-2 text-destructive">
                <CircleX aria-hidden className="mt-0.5 size-4 shrink-0" />
                {t("verify.wrongProject", { project: checked.tag.project ?? "?" })}
              </p>
            )}
            {tagOk && (
              <p className="flex gap-2 text-emerald-700 dark:text-emerald-400">
                <CircleCheck aria-hidden className="mt-0.5 size-4 shrink-0" />
                {w.on ? t("verify.live") : t("verify.found")}
              </p>
            )}
            {tagOk && !checked.tag.key && <Note>{t("verify.noKey")}</Note>}
            {tagOk && !checked.tag.in_head && <Note>{t("verify.notHead")}</Note>}
            {checked && !checked.error && checked.csp_blocked.length > 0 && <Note>{t("warn.csp", { hosts, directives: checked.csp_blocked.join(", ") })}</Note>}
          </div>
        </Step>
      </ol>
    </section>
  );
}
