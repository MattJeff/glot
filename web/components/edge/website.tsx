"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Link } from "@/i18n/navigation";
import type { Project } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { useErrorMessage } from "@/lib/use-error-message";
import { type EdgeSettings, edgeApi, edgeKeys, SNIPPET_CORNERS, type WebsiteSnippet } from "./api";
import { HreflangSnippet } from "./setup";

const CUSTOM = "selector";
type Where = (typeof SNIPPET_CORNERS)[number] | typeof CUSTOM;

/** The universal website tag (any site builder): on/off, auto-translation, switcher placement, languages, the tag. */
export function WebsiteTag({ project, settings }: { project: Project; settings: EdgeSettings }) {
  const t = useTranslations("edge.website");
  const slug = project.slug;
  const qc = useQueryClient();
  const message = useErrorMessage();
  const origin = useOrigin();
  const localeName = useLocaleName();
  const w = settings.website_snippet;
  const [on, setOn] = useState(w.on);
  const [auto, setAuto] = useState(w.auto_translate);
  const corner = (SNIPPET_CORNERS as readonly string[]).includes(w.position);
  const [where, setWhere] = useState<Where>(corner ? (w.position as Where) : CUSTOM);
  const [selector, setSelector] = useState(corner ? "" : w.position);
  const [locales, setLocales] = useState<string[]>(w.locales);
  const key = useQuery({ queryKey: edgeKeys.snippet(slug), queryFn: () => edgeApi.snippet(slug) });
  const domains = useQuery({ queryKey: edgeKeys.domains(slug), queryFn: () => edgeApi.domains(slug) });
  const seo = domains.data?.some((d) => d.mode === "subdomain" && d.verified_at && d.enabled);

  const save = useMutation({
    mutationFn: () => {
      const cur = qc.getQueryData<EdgeSettings>(edgeKeys.settings(slug)) ?? settings;
      const next: WebsiteSnippet = { on, auto_translate: auto, position: where === CUSTOM ? selector.trim() : where, locales };
      return edgeApi.putSettings(slug, { ...cur, website_snippet: next });
    },
    onSuccess: (r) => {
      qc.setQueryData(edgeKeys.settings(slug), r);
      toast.success(t("saved"));
    },
    onError: (e) => toast.error(message(e)),
  });

  const host = origin ? new URL(origin).host : "";
  const server = host.startsWith("app.") ? "" : ` data-server="${origin}"`;
  const tag = key.data
    ? `<script src="${origin ?? ""}/widget/glot.js" data-project="${slug}" data-key="${key.data.public_key}"${server} async></script>`
    : "";

  return (
    <div className="space-y-8">
      <section className="space-y-3">
        <div>
          <h2 className="font-medium">{t("title")}</h2>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{t("body")}</p>
        </div>
        {tag ? <CodeBlock label={t("tagLabel")} code={tag} /> : <Skeleton className="h-16 rounded-lg" />}
        <p className="text-xs text-muted-foreground">{t("tagHint")}</p>
      </section>

      <form
        className="space-y-5"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="space-y-1.5">
          <Label className="flex items-center gap-3 font-normal">
            <Switch checked={on} onCheckedChange={setOn} aria-describedby="website-on-hint" /> {t("on")}
          </Label>
          <p id="website-on-hint" className="text-xs text-muted-foreground">{t("onHint")}</p>
        </div>
        <div className="space-y-1.5">
          <Label className="flex items-center gap-3 font-normal">
            <Switch checked={auto} onCheckedChange={setAuto} aria-describedby="website-auto-hint" /> {t("auto")}
          </Label>
          <p id="website-auto-hint" className="text-xs text-muted-foreground">{t("autoHint")}</p>
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor="website-position">{t("position")}</Label>
            <Select value={where} onValueChange={(v) => setWhere(v as Where)}>
              <SelectTrigger id="website-position" className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {([...SNIPPET_CORNERS, CUSTOM] as const).map((p) => (
                  <SelectItem key={p} value={p}>
                    {t(`positions.${p}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {where === CUSTOM && (
            <div className="space-y-1.5">
              <Label htmlFor="website-selector">{t("selector")}</Label>
              <Input id="website-selector" required value={selector} onChange={(e) => setSelector(e.target.value)} placeholder="#SITE_HEADER nav" dir="ltr" translate="no" className="font-mono" aria-describedby="website-selector-hint" />
              <p id="website-selector-hint" className="text-xs text-muted-foreground">{t("selectorHint")}</p>
            </div>
          )}
        </div>
        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">{t("locales")}</legend>
          <p className="text-xs text-muted-foreground">{t("localesHint")}</p>
          <div className="flex flex-wrap gap-4">
            {project.target_locales.map((l) => (
              <Label key={l} className="flex items-center gap-2 font-normal">
                <Checkbox checked={locales.includes(l)} onCheckedChange={(c) => setLocales(c ? [...locales, l] : locales.filter((x) => x !== l))} />
                {localeName(l)} <span className="font-mono text-xs text-muted-foreground">{l}</span>
              </Label>
            ))}
          </div>
        </fieldset>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />}
          {t("save")}
        </Button>
      </form>

      <section className="space-y-3 rounded-xl border p-5">
        <div>
          <h2 className="font-medium">{t("seo.title")}</h2>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{seo ? t("seo.ready") : t("seo.body")}</p>
        </div>
        {!seo && (
          <Button variant="outline" size="sm" asChild>
            <Link href={`/app/${slug}/edge?tab=setup`}>{t("seo.cta")}</Link>
          </Button>
        )}
      </section>
      {seo && <HreflangSnippet slug={slug} />}
    </div>
  );
}
