"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useErrorMessage } from "@/lib/use-error-message";
import { type EdgeSettings, edgeApi, edgeKeys, type Rendering } from "./api";

const lines = (s: string) => s.split("\n").map((l) => l.trim()).filter(Boolean);
type ListField = "discover_include" | "discover_exclude" | "selectors_include" | "selectors_exclude" | "critical_patterns" | "sensitive_patterns";

/** Where segments may be discovered, what is never touched, critical/sensitive pages, auto-translation. */
export function EdgeSettingsForm({ slug, settings }: { slug: string; settings: EdgeSettings }) {
  const t = useTranslations("edge.settings");
  const qc = useQueryClient();
  const message = useErrorMessage();
  const [auto, setAuto] = useState(settings.auto_translate);
  const [budget, setBudget] = useState(settings.monthly_budget_usd === null ? "" : String(settings.monthly_budget_usd));
  const [rendering, setRendering] = useState<Rendering>(settings.rendering);
  const [cap, setCap] = useState(String(settings.daily_cap));
  const [threshold, setThreshold] = useState(String(Math.round(settings.critical_threshold * 100)));
  const [text, setText] = useState<Record<ListField, string>>(() => ({
    discover_include: settings.discover_include.join("\n"),
    discover_exclude: settings.discover_exclude.join("\n"),
    selectors_include: settings.selectors_include.join("\n"),
    selectors_exclude: settings.selectors_exclude.join("\n"),
    critical_patterns: settings.critical_patterns.join("\n"),
    sensitive_patterns: settings.sensitive_patterns.join("\n"),
  }));

  const save = useMutation({
    // Kill switch state comes from the cache: the header switch may have changed it since this form mounted.
    mutationFn: () =>
      edgeApi.putSettings(slug, {
        enabled: qc.getQueryData<EdgeSettings>(edgeKeys.settings(slug))?.enabled ?? settings.enabled,
        auto_translate: auto,
        monthly_budget_usd: budget === "" ? null : Number(budget),
        rendering,
        daily_cap: Number(cap),
        critical_threshold: Number(threshold) / 100,
        discover_include: lines(text.discover_include),
        discover_exclude: lines(text.discover_exclude),
        selectors_include: lines(text.selectors_include),
        selectors_exclude: lines(text.selectors_exclude),
        critical_patterns: lines(text.critical_patterns),
        sensitive_patterns: lines(text.sensitive_patterns),
      }),
    onSuccess: (r) => {
      qc.setQueryData(edgeKeys.settings(slug), r);
      qc.invalidateQueries({ queryKey: edgeKeys.coverage(slug) });
      toast.success(t("saved"));
    },
    onError: (e) => toast.error(message(e)),
  });

  const field = (id: string, label: string, hint: string, control: React.ReactNode) => (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {control}
      <p id={`${id}-hint`} className="text-xs text-muted-foreground">
        {hint}
      </p>
    </div>
  );
  const area = (k: ListField, label: string, hint: string, placeholder: string) =>
    field(
      `edge-${k}`,
      label,
      hint,
      <Textarea
        id={`edge-${k}`}
        aria-describedby={`edge-${k}-hint`}
        rows={4}
        value={text[k]}
        onChange={(e) => setText({ ...text, [k]: e.target.value })}
        placeholder={placeholder}
        dir="ltr"
        translate="no"
        className="font-mono text-xs"
      />,
    );

  return (
    <form
      className="space-y-10"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <section className="space-y-4">
        <h2 className="font-medium">{t("translate.title")}</h2>
        <label className="flex items-center gap-3 text-sm">
          <Switch checked={auto} onCheckedChange={setAuto} /> {t("translate.auto")}
        </label>
        <p className="text-xs text-muted-foreground">{t("translate.autoHint")}</p>
        <div className="grid gap-4 sm:grid-cols-2">
          {field(
            "edge-budget",
            t("translate.budget"),
            t("translate.budgetHint"),
            <Input id="edge-budget" type="number" min={0} step="0.01" value={budget} onChange={(e) => setBudget(e.target.value)} className="w-40 font-mono" />,
          )}
          {field(
            "edge-rendering",
            t("rendering.label"),
            t(`rendering.${rendering}Hint`),
            <Select value={rendering} onValueChange={(v) => setRendering(v as Rendering)}>
              <SelectTrigger id="edge-rendering" className="w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(["auto", "origin", "prerender"] as const).map((r) => (
                  <SelectItem key={r} value={r}>
                    {t(`rendering.${r}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>,
          )}
        </div>
      </section>

      <section className="space-y-4">
        <div>
          <h2 className="font-medium">{t("discovery.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("discovery.body")}</p>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          {area("discover_include", t("discovery.include"), t("discovery.includeHint"), "/\n/pricing\n/blog/*")}
          {area("discover_exclude", t("discovery.exclude"), t("discovery.excludeHint"), "/account/*\n/app/*")}
          {area("selectors_include", t("discovery.selectorsInclude"), t("discovery.selectorsIncludeHint"), "main\n#content")}
          {area("selectors_exclude", t("discovery.selectors"), t("discovery.selectorsHint"), ".user-name\n[data-glot-user]\n#comments")}
          {field(
            "edge-cap",
            t("discovery.cap"),
            t("discovery.capHint"),
            <Input id="edge-cap" type="number" min={0} max={100000} value={cap} onChange={(e) => setCap(e.target.value)} className="w-40 font-mono" />,
          )}
        </div>
      </section>

      <section className="space-y-4">
        <div>
          <h2 className="font-medium">{t("critical.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("critical.body")}</p>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          {area("critical_patterns", t("critical.patterns"), t("critical.patternsHint"), "/\n/signup*\n/pricing*\n/checkout*")}
          {area("sensitive_patterns", t("critical.sensitive"), t("critical.sensitiveHint"), "/checkout*\n/account*")}
          {field(
            "edge-threshold",
            t("critical.threshold"),
            t("critical.thresholdHint"),
            <Input id="edge-threshold" type="number" min={0} max={100} value={threshold} onChange={(e) => setThreshold(e.target.value)} className="w-28 font-mono" />,
          )}
        </div>
      </section>

      <Button type="submit" disabled={save.isPending}>
        {save.isPending && <Loader2 className="animate-spin" />}
        {t("save")}
      </Button>
    </form>
  );
}
