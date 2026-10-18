"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Plus, X } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { parseLocales } from "@/components/create-project-dialog";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useRouter } from "@/i18n/navigation";
import { api, type LlmSettings, type PreviewSettings, type Project, type Provider, qk } from "@/lib/api";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";

const PROVIDERS: Provider[] = ["anthropic", "openai", "openai-compatible", "mock"];

export default function SettingsPage() {
  const project = useProject(useSlug());
  if (project.isError) return <ErrorState error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <Skeleton className="mx-auto h-[70vh] max-w-3xl rounded-xl" />;
  return <SettingsForm key={project.data.id} project={project.data} />;
}

function Section({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return (
    <section className="grid gap-4 border-b py-8 md:grid-cols-[14rem_1fr] md:gap-10">
      <div>
        <h2 className="font-medium">{title}</h2>
        {description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}
      </div>
      <div className="min-w-0 space-y-4">{children}</div>
    </section>
  );
}

function Field({ id, label, hint, children }: { id: string; label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}

function SettingsForm({ project: p }: { project: Project }) {
  const t = useTranslations("settings");
  const tc = useTranslations("common");
  const localeName = useLocaleName();
  const qc = useQueryClient();
  const router = useRouter();
  const origin = useOrigin() ?? "";

  const [name, setName] = useState(p.name);
  const [targets, setTargets] = useState(p.target_locales);
  const [newLocale, setNewLocale] = useState("");
  const [style, setStyle] = useState(p.style ?? "");
  const [cdn, setCdn] = useState(p.public_cdn);
  const [llm, setLlm] = useState<LlmSettings>({
    provider: p.llm.provider,
    model: p.llm.model,
    fallback_model: p.llm.fallback_model ?? "",
    judge_model: p.llm.judge_model ?? "",
    api_key: "",
    // ponytail: Project.llm doesn't expose base_url in the contract, so it can't be prefilled.
    base_url: "",
    budget_usd: p.llm.budget_usd,
    anthropic_workspace_id: p.llm.anthropic_workspace_id ?? "",
  });
  const [headers, setHeaders] = useState(
    Object.entries(p.llm.extra_headers ?? {})
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n"),
  );
  const [confirm, setConfirm] = useState("");
  const set = <K extends keyof LlmSettings>(k: K, v: LlmSettings[K]) => setLlm((s) => ({ ...s, [k]: v }));

  const save = useMutation({
    mutationFn: () =>
      api.updateProject(p.slug, {
        name,
        target_locales: targets,
        style,
        public_cdn: cdn,
        llm: {
          provider: llm.provider,
          model: llm.model,
          fallback_model: llm.fallback_model || null,
          judge_model: llm.judge_model || null,
          budget_usd: llm.budget_usd ?? null,
          anthropic_workspace_id: llm.anthropic_workspace_id ?? "",
          extra_headers: Object.fromEntries(
            headers
              .split("\n")
              .filter((l) => l.includes(":"))
              .map((l) => [l.slice(0, l.indexOf(":")).trim(), l.slice(l.indexOf(":") + 1).trim()])
              .filter(([k]) => k),
          ),
          ...(llm.api_key ? { api_key: llm.api_key } : {}),
          ...(llm.base_url ? { base_url: llm.base_url } : {}),
        },
      }),
    onSuccess: (next) => {
      qc.setQueryData(qk.project(p.slug), next);
      qc.invalidateQueries({ queryKey: qk.projects });
      set("api_key", "");
      toast.success(t("saved"));
    },
  });

  const remove = useMutation({
    mutationFn: () => api.deleteProject(p.slug),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.projects });
      toast.success(t("danger.deleted"));
      router.replace("/app");
    },
  });

  const addLocales = () => {
    const add = parseLocales(newLocale).filter((l) => l !== p.source_locale && !targets.includes(l));
    if (add.length) setTargets([...targets, ...add]);
    setNewLocale("");
  };

  const cdnUrl = `${origin}/cdn/${p.slug}/${targets[0] ?? p.source_locale}.json`;

  return (
    <form
      className="mx-auto max-w-4xl"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <PageHeader title={t("title")} subtitle={<span className="font-mono">{p.slug}</span>} />

      <Section title={t("general.title")}>
        <Field id="s-name" label={t("general.name")}>
          <Input id="s-name" value={name} onChange={(e) => setName(e.target.value)} required />
        </Field>
        <Field id="s-slug" label={t("general.slug")}>
          <Input id="s-slug" value={p.slug} readOnly disabled dir="ltr" className="font-mono" />
        </Field>
      </Section>

      <Section title={t("locales.title")} description={t("locales.description")}>
        <Field id="s-source" label={t("locales.source")}>
          <Input id="s-source" value={p.source_locale} readOnly disabled dir="ltr" className="w-32 font-mono" />
        </Field>
        <div className="space-y-1.5">
          <Label htmlFor="s-add-locale">{t("locales.targets")}</Label>
          <div className="flex flex-wrap gap-1.5">
            {targets.map((l) => (
              <span key={l} className="inline-flex h-7 items-center gap-1.5 rounded-md border bg-muted/40 ps-2 pe-1 text-sm">
                <span className="font-mono text-xs">{l}</span>
                <span className="text-muted-foreground capitalize">{localeName(l)}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  aria-label={t("locales.remove", { locale: l })}
                  onClick={() => setTargets(targets.filter((x) => x !== l))}
                >
                  <X />
                </Button>
              </span>
            ))}
          </div>
          <div className="flex max-w-xs gap-2 pt-1">
            <Input
              id="s-add-locale"
              value={newLocale}
              dir="ltr"
              placeholder={t("locales.addPlaceholder")}
              className="font-mono"
              onChange={(e) => setNewLocale(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addLocales();
                }
              }}
            />
            <Button type="button" variant="outline" onClick={addLocales}>
              <Plus /> {t("locales.add")}
            </Button>
          </div>
        </div>
      </Section>

      <Section title={t("llm.title")} description={t("llm.description")}>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field id="s-provider" label={t("llm.provider")}>
            <Select value={llm.provider} onValueChange={(v) => set("provider", v as Provider)}>
              <SelectTrigger id="s-provider" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PROVIDERS.map((pr) => (
                  <SelectItem key={pr} value={pr}>
                    {t(`llm.providers.${pr}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field id="s-model" label={t("llm.model")}>
            <Input id="s-model" value={llm.model} onChange={(e) => set("model", e.target.value)} required dir="ltr" className="font-mono" />
          </Field>
          <Field id="s-fallback" label={t("llm.fallback")} hint={t("llm.fallbackHint")}>
            <Input id="s-fallback" value={llm.fallback_model ?? ""} onChange={(e) => set("fallback_model", e.target.value)} dir="ltr" className="font-mono" />
          </Field>
          <Field id="s-judge" label={t("llm.judge")} hint={t("llm.judgeHint")}>
            <Input id="s-judge" value={llm.judge_model ?? ""} onChange={(e) => set("judge_model", e.target.value)} dir="ltr" className="font-mono" />
          </Field>
        </div>
        <Field id="s-key" label={t("llm.apiKey")} hint={p.llm.has_api_key ? t("llm.apiKeySet") : t("llm.apiKeyUnset")}>
          <Input
            id="s-key"
            type="password"
            autoComplete="off"
            value={llm.api_key ?? ""}
            onChange={(e) => set("api_key", e.target.value)}
            placeholder={p.llm.has_api_key ? "••••••••••••" : ""}
            dir="ltr"
            className="font-mono"
          />
        </Field>
        {llm.provider === "openai-compatible" && (
          <Field id="s-base" label={t("llm.baseUrl")} hint={t("llm.baseUrlHint")}>
            <Input
              id="s-base"
              type="url"
              value={llm.base_url ?? ""}
              onChange={(e) => set("base_url", e.target.value)}
              placeholder="http://localhost:11434/v1"
              dir="ltr"
              className="font-mono"
            />
          </Field>
        )}
        {llm.provider === "anthropic" && (
          <Field id="s-workspace" label={t("llm.workspaceId")} hint={t("llm.workspaceIdHint")}>
            <Input
              id="s-workspace"
              value={llm.anthropic_workspace_id ?? ""}
              onChange={(e) => set("anthropic_workspace_id", e.target.value)}
              placeholder="wrkspc_…"
              dir="ltr"
              className="font-mono"
            />
          </Field>
        )}
        <Field id="s-headers" label={t("llm.extraHeaders")} hint={t("llm.extraHeadersHint")}>
          <Textarea
            id="s-headers"
            rows={2}
            value={headers}
            onChange={(e) => setHeaders(e.target.value)}
            placeholder="x-team: growth"
            translate="no"
            dir="ltr"
            className="font-mono"
          />
        </Field>
        <Field id="s-budget" label={t("llm.budget")} hint={t("llm.budgetHint")}>
          <Input
            id="s-budget"
            type="number"
            min={0}
            step="0.01"
            value={llm.budget_usd ?? ""}
            onChange={(e) => set("budget_usd", e.target.value === "" ? null : Number(e.target.value))}
            className="w-40 font-mono"
          />
        </Field>
        <Field id="s-style" label={t("llm.style")}>
          <Textarea id="s-style" rows={3} value={style} onChange={(e) => setStyle(e.target.value)} placeholder={t("llm.stylePlaceholder")} />
        </Field>
      </Section>

      <Section title={t("cdn.title")} description={t("cdn.description")}>
        <label className="flex items-center gap-3 text-sm">
          <Switch checked={cdn} onCheckedChange={setCdn} /> {t("cdn.enable")}
        </label>
        {cdn && <CodeBlock label={t("cdn.example")} code={`curl ${cdnUrl}`} />}
      </Section>

      <PreviewSection slug={p.slug} />

      <div className="sticky bottom-0 z-10 -mx-4 flex justify-end gap-2 border-t bg-background/90 px-4 py-3 backdrop-blur">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />}
          {save.isPending ? tc("saving") : tc("save")}
        </Button>
      </div>

      <section className="mt-10 rounded-xl border border-destructive/40 p-5">
        <h2 className="font-medium text-destructive">{t("danger.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("danger.description")}</p>
        <AlertDialog onOpenChange={() => setConfirm("")}>
          <AlertDialogTrigger asChild>
            <Button type="button" variant="destructive" className="mt-4">
              {t("danger.delete")}
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("danger.confirmTitle", { name: p.name })}</AlertDialogTitle>
              <AlertDialogDescription>{t("danger.confirmBody", { slug: p.slug })}</AlertDialogDescription>
            </AlertDialogHeader>
            <Input
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              dir="ltr"
              className="font-mono"
              aria-label={t("danger.confirmBody", { slug: p.slug })}
              autoComplete="off"
            />
            <AlertDialogFooter>
              <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
              <Button variant="destructive" disabled={confirm !== p.slug || remove.isPending} onClick={() => remove.mutate()}>
                {remove.isPending && <Loader2 className="animate-spin" />}
                {t("danger.delete")}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </section>
    </form>
  );
}

function PreviewSection({ slug }: { slug: string }) {
  const t = useTranslations("settings.preview");
  const preview = useQuery({ queryKey: qk.preview(slug), queryFn: () => api.preview(slug) });
  return (
    <Section title={t("title")} description={t("description")}>
      {preview.data ? <PreviewFields key={JSON.stringify(preview.data)} slug={slug} initial={preview.data} /> : <Skeleton className="h-40" />}
    </Section>
  );
}

function PreviewFields({ slug, initial }: { slug: string; initial: PreviewSettings }) {
  const t = useTranslations("settings.preview");
  const tc = useTranslations("common");
  const qc = useQueryClient();
  const [base, setBase] = useState(initial.base_url ?? "");
  const [pattern, setPattern] = useState(initial.url_pattern);
  const [routes, setRoutes] = useState(initial.routes.join("\n"));
  const save = useMutation({
    mutationFn: () =>
      api.updatePreview(slug, { base_url: base || null, url_pattern: pattern, routes: routes.split(/[\n,]/) }),
    onSuccess: (next) => {
      qc.setQueryData(qk.preview(slug), next);
      toast.success(t("saved"));
    },
  });
  return (
    <div id="preview" className="scroll-mt-20 space-y-4">
      <Field id="s-preview-base" label={t("baseUrl")} hint={t("baseUrlHint")}>
        <Input id="s-preview-base" type="url" value={base} onChange={(e) => setBase(e.target.value)} placeholder="http://localhost:3000" dir="ltr" className="font-mono" />
      </Field>
      <Field id="s-preview-pattern" label={t("pattern")} hint={t("patternHint")}>
        <Input id="s-preview-pattern" value={pattern} onChange={(e) => setPattern(e.target.value)} required dir="ltr" className="font-mono" />
      </Field>
      <Field id="s-preview-routes" label={t("routes")} hint={t("routesHint")}>
        <Textarea id="s-preview-routes" rows={3} value={routes} onChange={(e) => setRoutes(e.target.value)} dir="ltr" className="font-mono" />
      </Field>
      <Button type="button" variant="outline" disabled={save.isPending} onClick={() => save.mutate()}>
        {save.isPending && <Loader2 className="animate-spin" />}
        {save.isPending ? tc("saving") : t("save")}
      </Button>
    </div>
  );
}
