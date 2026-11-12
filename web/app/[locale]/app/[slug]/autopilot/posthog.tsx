"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, PlugZap, RefreshCw, Unplug } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { api, qk, type PosthogConnect, type PosthogTested, type PosthogView } from "@/lib/api";
import { Section } from "../insights/parts";

const REGIONS = { us: "https://us.posthog.com", eu: "https://eu.posthog.com" } as const;
const NONE = "__none__";

/** PostHog import: connect (tested before anything is stored), sync, disconnect. The key is write-only. */
export function PosthogSource({ slug, v, sourceLocale }: { slug: string; v: PosthogView; sourceLocale: string }) {
  const t = useTranslations("autopilot.posthog");
  const format = useFormatter();
  const qc = useQueryClient();
  const region = v.host === REGIONS.eu ? "eu" : v.host && v.host !== REGIONS.us ? "custom" : "us";
  const [b, setB] = useState({
    region,
    url: region === "custom" ? (v.host ?? "") : "",
    project_id: v.project_id ?? "",
    api_key: "",
    conversion_event: v.conversion_event ?? "",
    default_locale: v.connected ? (v.default_locale ?? "") : sourceLocale,
    backfill_days: v.backfill_days,
  });
  const [tested, setTested] = useState<PosthogTested | null>(null);
  const set = (patch: Partial<typeof b>) => setB((x) => ({ ...x, ...patch }));
  const body = (): PosthogConnect => ({
    host: b.region === "custom" ? b.url.trim() : b.region,
    project_id: b.project_id.trim(),
    api_key: b.api_key.trim() || undefined,
    conversion_event: b.conversion_event || null,
    default_locale: b.default_locale.trim() || null,
    backfill_days: b.backfill_days,
  });
  const refresh = () => qc.invalidateQueries({ queryKey: qk.autopilot(slug) });
  const test = useMutation({
    mutationFn: () => api.posthogTest(slug, body()),
    onSuccess: (r) => {
      setTested(r);
      if (!b.conversion_event && r.suggested_event) set({ conversion_event: r.suggested_event });
    },
    onError: (e) => toast.error(e.message),
  });
  const sync = useMutation({
    mutationFn: () => api.posthogSync(slug),
    onSuccess: (r) => {
      refresh();
      toast.success(t("imported", { rows: r.rows, queries: r.queries }));
    },
    onError: (e) => {
      refresh();
      toast.error(e.message);
    },
  });
  const save = useMutation({
    mutationFn: () => api.posthogSave(slug, body()),
    onSuccess: () => {
      set({ api_key: "" });
      refresh();
      sync.mutate();
    },
    onError: (e) => toast.error(e.message),
  });
  const disconnect = useMutation({
    mutationFn: () => api.posthogDisconnect(slug),
    onSuccess: () => {
      setTested(null);
      refresh();
      toast.success(t("disconnected"));
    },
    onError: (e) => toast.error(e.message),
  });
  const busy = test.isPending || save.isPending || sync.isPending || disconnect.isPending;
  const date = (s: string) => format.dateTime(new Date(s), { day: "numeric", month: "short" });
  const events = tested?.events.map(([name]) => name) ?? (v.conversion_event ? [v.conversion_event] : []);

  return (
    <Section title={t("title")} subtitle={t("subtitle")}>
      <div className="mb-4 space-y-1 text-sm">
        <p>{v.connected ? t("connected", { project: v.project_id ?? "", host: v.host ?? "" }) : t("notConnected")}</p>
        {v.connected &&
          (v.synced_from && v.synced_to && v.last_sync_at ? (
            <p className="text-muted-foreground">
              {t("synced", { from: date(v.synced_from), to: date(v.synced_to), when: format.relativeTime(new Date(v.last_sync_at)) })}
            </p>
          ) : (
            <p className="text-muted-foreground">{t("never")}</p>
          ))}
        {v.last_error && <p className="text-destructive">{t("error", { error: v.last_error })}</p>}
      </div>
      <form
        className="grid gap-3 sm:grid-cols-2"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="grid gap-1.5">
          <Label htmlFor="ph-region">{t("host")}</Label>
          <Select value={b.region} onValueChange={(r) => set({ region: r as typeof b.region })}>
            <SelectTrigger id="ph-region" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="us">{t("us")}</SelectItem>
              <SelectItem value="eu">{t("eu")}</SelectItem>
              <SelectItem value="custom">{t("custom")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {b.region === "custom" ? (
          <div className="grid gap-1.5">
            <Label htmlFor="ph-url">{t("url")}</Label>
            <Input id="ph-url" type="url" value={b.url} onChange={(e) => set({ url: e.target.value })} placeholder="https://posthog.example.com" required translate="no" />
          </div>
        ) : (
          <div />
        )}
        <div className="grid gap-1.5">
          <Label htmlFor="ph-project">{t("project")}</Label>
          <Input id="ph-project" inputMode="numeric" value={b.project_id} onChange={(e) => set({ project_id: e.target.value })} required className="font-mono" translate="no" />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ph-key">{t("key")}</Label>
          <Input
            id="ph-key"
            type="password"
            autoComplete="off"
            value={b.api_key}
            onChange={(e) => set({ api_key: e.target.value })}
            placeholder={v.connected ? t("keySaved") : t("keyPlaceholder")}
            required={!v.connected}
            className="font-mono"
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ph-event">{t("event")}</Label>
          {events.length > 0 ? (
            <Select value={b.conversion_event || NONE} onValueChange={(e) => set({ conversion_event: e === NONE ? "" : e })}>
              <SelectTrigger id="ph-event" className="w-full font-mono">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>{t("eventNone")}</SelectItem>
                {events.map((e) => (
                  <SelectItem key={e} value={e} className="font-mono">
                    {e}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <Input id="ph-event" value={b.conversion_event} onChange={(e) => set({ conversion_event: e.target.value })} className="font-mono" translate="no" />
          )}
          <p className="text-xs text-muted-foreground">{t("eventHint")}</p>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ph-locale">{t("locale")}</Label>
          <Input id="ph-locale" value={b.default_locale} onChange={(e) => set({ default_locale: e.target.value })} className="font-mono" translate="no" />
          <p className="text-xs text-muted-foreground">{t("localeHint")}</p>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="ph-days">{t("backfill")}</Label>
          <Input id="ph-days" type="number" min={7} max={365} value={b.backfill_days} onChange={(e) => set({ backfill_days: Number(e.target.value) })} />
        </div>
        {tested && (
          <p className="self-end rounded-md bg-muted/50 p-3 text-sm sm:col-span-2">
            {t("tested", {
              pageviews: format.number(tested.pageviews_30d),
              share: format.number(tested.pageviews_30d ? tested.with_language_30d / tested.pageviews_30d : 0, { style: "percent" }),
            })}
          </p>
        )}
        <div className="flex flex-wrap gap-2 sm:col-span-2">
          <Button type="button" variant="outline" onClick={() => test.mutate()} disabled={busy}>
            {test.isPending ? <Loader2 className="animate-spin" /> : <PlugZap />} {t("test")}
          </Button>
          <Button type="submit" disabled={busy}>
            {save.isPending || sync.isPending ? <Loader2 className="animate-spin" /> : null} {sync.isPending ? t("syncing") : t("save")}
          </Button>
          {v.connected && (
            <>
              <Button type="button" variant="outline" onClick={() => sync.mutate()} disabled={busy}>
                <RefreshCw /> {t("sync")}
              </Button>
              <Button type="button" variant="ghost" onClick={() => disconnect.mutate()} disabled={busy}>
                <Unplug /> {t("disconnect")}
              </Button>
            </>
          )}
        </div>
      </form>
      <details className="mt-4 text-sm text-muted-foreground">
        <summary className="cursor-pointer select-none underline-offset-4 hover:underline">{t("steps")}</summary>
        <ol className="mt-2 list-decimal space-y-1 ps-5">
          <li>{t("step1")}</li>
          <li>{t("step2")}</li>
          <li>{t("step3")}</li>
          <li>{t("step4")}</li>
        </ol>
      </details>
      <p className="mt-3 text-xs text-muted-foreground">{t("privacy")}</p>
    </Section>
  );
}
