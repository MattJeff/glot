"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Plus, X } from "lucide-react";
import { useTranslations } from "next-intl";
import { cloneElement, useState } from "react";
import { toast } from "sonner";
import {
  CodeBlock,
  ErrorState,
  PageHeader,
  useLocaleName,
} from "@/components/common";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Link, useRouter } from "@/i18n/navigation";
import {
  api,
  type LlmSettings,
  type PreviewSettings,
  type Project,
  type Provider,
  qk,
} from "@/lib/api";
import { useOrigin, useProject, useSlug } from "@/lib/hooks";

const PROVIDERS: Provider[] = [
  "anthropic",
  "openai",
  "openai-compatible",
  "mock",
];

export default function SettingsPage() {
  const slug = useSlug();
  const project = useProject(slug);
  const preview = useQuery({
    queryKey: qk.preview(slug),
    queryFn: () => api.preview(slug),
  });
  const failed = project.isError ? project : preview.isError ? preview : null;
  if (failed)
    return <ErrorState error={failed.error} onRetry={() => failed.refetch()} />;
  if (!project.data || !preview.data)
    return <Skeleton className="mx-auto h-[70vh] max-w-3xl rounded-xl" />;
  return (
    <SettingsForm
      key={project.data.id}
      project={project.data}
      preview={preview.data}
    />
  );
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="grid gap-4 border-b py-8 md:grid-cols-[14rem_1fr] md:gap-10">
      <div>
        <h2 className="font-medium">{title}</h2>
        {description && (
          <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      <div className="min-w-0 space-y-4">{children}</div>
    </section>
  );
}

/** Label + control + description, wired with aria-describedby so screen readers read the hint. */
function Field({
  id,
  label,
  hint,
  children,
}: {
  id: string;
  label: string;
  hint?: string;
  children: React.ReactElement<{ "aria-describedby"?: string }>;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {hint
        ? cloneElement(children, { "aria-describedby": `${id}-hint` })
        : children}
      {hint && (
        <p
          id={`${id}-hint`}
          className="text-xs text-pretty text-muted-foreground"
        >
          {hint}
        </p>
      )}
    </div>
  );
}

function SettingsForm({
  project: p,
  preview,
}: {
  project: Project;
  preview: PreviewSettings;
}) {
  const t = useTranslations("settings");
  const tc = useTranslations("common");
  const ts = useTranslations("shell");
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
  const [site, setSite] = useState(preview.base_url ?? "");
  const [pattern, setPattern] = useState(preview.url_pattern);
  const [routes, setRoutes] = useState(preview.routes.join("\n"));
  // Same query as Review's connection probe: it also reads how the site switches language (hreflang).
  const probe = useQuery({
    queryKey: ["review-probe", p.slug],
    queryFn: () => api.reviewProbe(p.slug),
    enabled: !!preview.base_url,
    staleTime: 60_000,
    retry: false,
  });
  const detected = probe.data?.locale_scheme;
  const detectedHow =
    detected?.kind === "query"
      ? `?${detected.pattern?.match(/[?&]([^=]+)=\{locale\}/)?.[1] ?? "lang"}=`
      : `/${detected?.locales.find((l) => l !== detected.default_locale) ?? "en"}/…`;
  // ponytail: primary subtags only (fr-FR = fr); pt-BR vs pt-PT goes unflagged.
  const sameLang = (a: string, b: string) =>
    a.split("-")[0].toLowerCase() === b.split("-")[0].toLowerCase();
  const [confirm, setConfirm] = useState("");
  const [nextSource, setNextSource] = useState<string | null>(null);
  const set = <K extends keyof LlmSettings>(k: K, v: LlmSettings[K]) =>
    setLlm((s) => ({ ...s, [k]: v }));

  const save = useMutation({
    mutationFn: async () => {
      // One save button for the whole page: the site URL used to need its own button, and got lost.
      const nextPreview = await api.updatePreview(p.slug, {
        base_url: siteUrl(site),
        url_pattern: pattern,
        routes: routes.split(/[\n,]/),
      });
      qc.setQueryData(qk.preview(p.slug), nextPreview);
      qc.invalidateQueries({ queryKey: ["review-probe", p.slug] });
      setSite(nextPreview.base_url ?? "");
      return api.updateProject(p.slug, {
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
              .map((l) => [
                l.slice(0, l.indexOf(":")).trim(),
                l.slice(l.indexOf(":") + 1).trim(),
              ])
              .filter(([k]) => k),
          ),
          ...(llm.api_key ? { api_key: llm.api_key } : {}),
          ...(llm.base_url ? { base_url: llm.base_url } : {}),
        },
      });
    },
    onSuccess: (next) => {
      qc.setQueryData(qk.project(p.slug), next);
      qc.invalidateQueries({ queryKey: qk.projects });
      set("api_key", "");
      toast.success(t("saved"));
    },
  });

  const changeSource = useMutation({
    // Unsaved target edits ride along; the old source stays as a target.
    mutationFn: (to: string) =>
      api.updateProject(p.slug, {
        source_locale: to,
        target_locales: [...targets.filter((l) => l !== to), p.source_locale],
      }),
    onSuccess: (next) => {
      qc.setQueryData(qk.project(p.slug), next);
      qc.invalidateQueries({ queryKey: qk.projects });
      setTargets(next.target_locales);
      setNextSource(null);
      toast.success(t("locales.changed"));
    },
  });
  const nextStats = nextSource ? p.stats.locales[nextSource] : undefined;
  const missing =
    p.stats.keys - (nextStats ? nextStats.translated + nextStats.stale : 0);

  const remove = useMutation({
    mutationFn: () => api.deleteProject(p.slug),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.projects });
      toast.success(t("danger.deleted"));
      router.replace("/app");
    },
  });

  const addLocales = () => {
    const add = parseLocales(newLocale).filter(
      (l) => l !== p.source_locale && !targets.includes(l),
    );
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
      <PageHeader
        title={t("title")}
        eyebrow={<span className="font-mono">{p.slug}</span>}
        subtitle={t("subtitle")}
      />

      <Section title={t("general.title")}>
        <Field id="s-name" label={t("general.name")}>
          <Input
            id="s-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
        </Field>
        <Field
          id="s-slug"
          label={t("general.slug")}
          hint={t("general.slugHint")}
        >
          <Input
            id="s-slug"
            value={p.slug}
            readOnly
            disabled
            dir="ltr"
            className="font-mono"
          />
        </Field>
      </Section>

      <Section
        title={t("locales.title")}
        description={t("locales.description")}
      >
        <Field
          id="s-source"
          label={t("locales.source")}
          hint={t("locales.sourceHint")}
        >
          <Select
            value={p.source_locale}
            onValueChange={(v) => v !== p.source_locale && setNextSource(v)}
          >
            <SelectTrigger id="s-source" className="w-56">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[p.source_locale, ...targets].map((l) => (
                <SelectItem key={l} value={l}>
                  <span className="font-mono text-xs">{l}</span>
                  <span className="text-muted-foreground capitalize">
                    {localeName(l)}
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <AlertDialog
          open={nextSource !== null}
          onOpenChange={(open) => !open && setNextSource(null)}
        >
          {nextSource && (
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>
                  {t("locales.changeTitle", { to: localeName(nextSource) })}
                </AlertDialogTitle>
                <AlertDialogDescription>
                  {t("locales.changeBody", {
                    to: localeName(nextSource),
                    from: localeName(p.source_locale),
                  })}
                </AlertDialogDescription>
              </AlertDialogHeader>
              {missing > 0 && (
                <p className="text-sm font-medium">
                  {t("locales.changeMissing", {
                    count: missing,
                    to: localeName(nextSource),
                    from: localeName(p.source_locale),
                  })}
                </p>
              )}
              <p className="text-sm text-muted-foreground">
                {t("locales.changeCli", {
                  toCode: nextSource,
                  fromCode: p.source_locale,
                })}
              </p>
              <AlertDialogFooter>
                <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
                <Button
                  type="button"
                  disabled={changeSource.isPending}
                  onClick={() => changeSource.mutate(nextSource)}
                >
                  {changeSource.isPending && (
                    <Loader2 className="animate-spin" />
                  )}
                  {t("locales.changeConfirm")}
                </Button>
              </AlertDialogFooter>
            </AlertDialogContent>
          )}
        </AlertDialog>
        <div className="space-y-1.5">
          <Label htmlFor="s-add-locale">{t("locales.targets")}</Label>
          <div className="flex flex-wrap gap-1.5">
            {targets.map((l) => (
              <span
                key={l}
                className="inline-flex h-7 items-center gap-1.5 rounded-md border bg-muted/40 ps-2 pe-1 text-sm"
              >
                <span className="font-mono text-xs">{l}</span>
                <span className="text-muted-foreground capitalize">
                  {localeName(l)}
                </span>
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
            <Select
              value={llm.provider}
              onValueChange={(v) => set("provider", v as Provider)}
            >
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
          <Field id="s-model" label={t("llm.model")} hint={t("llm.modelHint")}>
            <Input
              id="s-model"
              value={llm.model}
              onChange={(e) => set("model", e.target.value)}
              required
              dir="ltr"
              className="font-mono"
            />
          </Field>
          <Field
            id="s-fallback"
            label={t("llm.fallback")}
            hint={t("llm.fallbackHint")}
          >
            <Input
              id="s-fallback"
              value={llm.fallback_model ?? ""}
              onChange={(e) => set("fallback_model", e.target.value)}
              dir="ltr"
              className="font-mono"
            />
          </Field>
          <Field id="s-judge" label={t("llm.judge")} hint={t("llm.judgeHint")}>
            <Input
              id="s-judge"
              value={llm.judge_model ?? ""}
              onChange={(e) => set("judge_model", e.target.value)}
              dir="ltr"
              className="font-mono"
            />
          </Field>
        </div>
        <Field
          id="s-key"
          label={t("llm.apiKey")}
          hint={p.llm.has_api_key ? t("llm.apiKeySet") : t("llm.apiKeyUnset")}
        >
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
          <Field
            id="s-base"
            label={t("llm.baseUrl")}
            hint={t("llm.baseUrlHint")}
          >
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
          <Field
            id="s-workspace"
            label={t("llm.workspaceId")}
            hint={t("llm.workspaceIdHint")}
          >
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
        <Field
          id="s-headers"
          label={t("llm.extraHeaders")}
          hint={t("llm.extraHeadersHint")}
        >
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
            onChange={(e) =>
              set(
                "budget_usd",
                e.target.value === "" ? null : Number(e.target.value),
              )
            }
            className="w-40 font-mono"
          />
        </Field>
        <Field id="s-style" label={t("llm.style")} hint={t("llm.styleHint")}>
          <Textarea
            id="s-style"
            rows={3}
            value={style}
            onChange={(e) => setStyle(e.target.value)}
            placeholder={t("llm.stylePlaceholder")}
          />
        </Field>
      </Section>

      <Section title={t("cdn.title")} description={t("cdn.description")}>
        <label className="flex items-center gap-3 text-sm">
          <Switch checked={cdn} onCheckedChange={setCdn} /> {t("cdn.enable")}
        </label>
        {cdn && <CodeBlock label={t("cdn.example")} code={`curl ${cdnUrl}`} />}
      </Section>

      <Section
        title={t("preview.title")}
        description={t("preview.description")}
      >
        <div id="preview" className="scroll-mt-20 space-y-4">
          <Field
            id="s-preview-base"
            label={t("preview.baseUrl")}
            hint={t("preview.baseUrlHint")}
          >
            <Input
              id="s-preview-base"
              inputMode="url"
              autoComplete="url"
              value={site}
              onChange={(e) => setSite(e.target.value)}
              placeholder="https://example.com"
              dir="ltr"
              className="font-mono"
            />
          </Field>
          {preview.base_url && (
            <p className="text-sm text-muted-foreground">
              {t("preview.usedBy")}{" "}
              {(["preview", "visual", "feedback", "experiments"] as const).map(
                (path, i) => (
                  <span key={path}>
                    {i > 0 && " · "}
                    <Link
                      href={`/app/${p.slug}/${path}`}
                      className="font-medium text-foreground underline underline-offset-4"
                    >
                      {ts(path)}
                    </Link>
                  </span>
                ),
              )}
            </p>
          )}
          <Field
            id="s-preview-pattern"
            label={t("preview.pattern")}
            hint={t("preview.patternHint")}
          >
            <Input
              id="s-preview-pattern"
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
              required
              dir="ltr"
              className="font-mono"
            />
          </Field>
          {detected?.pattern && detected.pattern !== pattern && (
            <div className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/40 p-3 text-sm">
              <span>{t("preview.detected", { how: detectedHow })}</span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => setPattern(detected.pattern!)}
              >
                {t("preview.useDetected", { pattern: detected.pattern })}
              </Button>
            </div>
          )}
          {detected?.kind === "subdomain" && (
            <p className="text-sm text-warning">{t("preview.subdomain")}</p>
          )}
          {detected?.default_locale &&
            !sameLang(detected.default_locale, p.source_locale) && (
              <p className="text-sm text-warning">
                {t("preview.defaultMismatch", {
                  site: localeName(detected.default_locale),
                  source: localeName(p.source_locale),
                })}
              </p>
            )}
          <Field
            id="s-preview-routes"
            label={t("preview.routes")}
            hint={t("preview.routesHint")}
          >
            <Textarea
              id="s-preview-routes"
              rows={3}
              value={routes}
              onChange={(e) => setRoutes(e.target.value)}
              dir="ltr"
              className="font-mono"
            />
          </Field>
        </div>
      </Section>

      <div className="sticky bottom-0 z-10 -mx-4 flex justify-end gap-2 border-t bg-background/90 px-4 py-3 backdrop-blur">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />}
          {save.isPending ? tc("saving") : tc("save")}
        </Button>
      </div>

      <section className="mt-10 rounded-xl border border-destructive/40 p-5">
        <h2 className="font-medium text-destructive">{t("danger.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("danger.description")}
        </p>
        <AlertDialog onOpenChange={() => setConfirm("")}>
          <AlertDialogTrigger asChild>
            <Button type="button" variant="destructive" className="mt-4">
              {t("danger.delete")}
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t("danger.confirmTitle", { name: p.name })}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {t("danger.confirmBody", { slug: p.slug })}
              </AlertDialogDescription>
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
              <Button
                variant="destructive"
                disabled={confirm !== p.slug || remove.isPending}
                onClick={() => remove.mutate()}
              >
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

/** `orizn.app` -> `https://orizn.app`, `localhost:3000` -> `http://localhost:3000`, empty -> null. */
function siteUrl(raw: string): string | null {
  const v = raw.trim().replace(/\/+$/, "");
  if (!v) return null;
  if (/^https?:\/\//i.test(v)) return v;
  return `${/^(localhost|127\.|\[::1\])/.test(v) ? "http" : "https"}://${v}`;
}
