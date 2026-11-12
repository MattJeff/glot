"use client";

// One connected store / site: what glot found (preview), what it is doing, what it published, what needs
// attention and how to fix it. Everything a merchant decides is here: approve, leave content out, pause,
// publish, undo.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { useSlug } from "@/lib/hooks";
import { type Check, type Connector, connectorKey, connectorsApi } from "./api";
import { key, OneClick } from "./connect";
import type { ConnectorDef } from "./types";

const sum = (o?: Record<string, number>) => Object.values(o ?? {}).reduce((a, b) => a + b, 0);
/** The platform's own content type name (`product_option`), readable. */
const typeLabel = (s: string) => s.replace(/[_-]/g, " ").replace(/^./, (c) => c.toUpperCase());

export function AccountRow({ account: a, def, oneClick }: { account: Connector; def: ConnectorDef; oneClick: boolean }) {
  const t = useTranslations("connectors");
  const format = useFormatter();
  const now = useNow({ updateInterval: 30_000 });
  const slug = useSlug();
  const qc = useQueryClient();
  const done = () => qc.invalidateQueries({ queryKey: connectorKey(slug) });
  const fail = (e: Error) => toast.error(e.message);
  const [panel, setPanel] = useState<"doctor" | "diff" | "history" | null>(null);
  const toggle = (p: typeof panel) => setPanel((cur) => (cur === p ? null : p));

  const sync = useMutation({
    mutationFn: () => connectorsApi.sync(slug, a.id),
    onSuccess: () => {
      done();
      toast.success(t("syncQueued"));
    },
    onError: fail,
  });
  const hold = useMutation({
    mutationFn: (h: Connector["hold"]) => connectorsApi.hold(slug, a.id, h),
    onSuccess: done,
    onError: fail,
  });
  const settings = useMutation({
    mutationFn: (s: Record<string, unknown>) => connectorsApi.update(slug, a.id, { settings: { ...a.settings, ...s } }),
    onSuccess: done,
    onError: fail,
  });
  const publish = useMutation({
    mutationFn: () => connectorsApi.publish(slug, a.id),
    onSuccess: (r) => {
      done();
      qc.invalidateQueries({ queryKey: ["connector-diff", a.id] });
      if (r.error) toast.warning(r.error);
      else toast.success(t("published"));
    },
    onError: fail,
  });
  const remove = useMutation({ mutationFn: () => connectorsApi.remove(slug, a.id), onSuccess: done, onError: fail });

  const busy = a.status === "queued" || a.status === "running";
  const translating = a.job && (a.job.status === "queued" || a.job.status === "running") ? a.job : null;
  const preview = a.hold === "translate" && !busy && a.stats.by_type !== undefined;
  const kept = a.stats.last_push?.kept ?? 0;
  const badge =
    a.status === "error" || busy ? t(`status.${a.status}`) : a.hold ? t(`hold.${a.hold}`) : t(`status.${a.status}`);

  return (
    <div className="mt-4 space-y-3 border-t pt-4 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{a.name}</span>
        <Badge variant={a.status === "error" ? "destructive" : "secondary"}>{badge}</Badge>
        {a.last_sync_at && (
          <span className="text-muted-foreground">
            {t("lastSync", { when: format.relativeTime(new Date(a.last_sync_at), now) })}
          </span>
        )}
      </div>

      {a.status === "running" && !translating && (
        <p className="text-muted-foreground tabular-nums" role="status">
          {t("progress.pull", { count: a.stats.pulled ?? 0 })}
        </p>
      )}
      {translating && (
        <div className="space-y-1" role="status">
          <p className="text-muted-foreground tabular-nums">
            {t("progress.translate", { done: translating.done, total: translating.total })}
          </p>
          <Progress value={translating.total ? (100 * translating.done) / translating.total : 0} />
        </div>
      )}

      {a.last_error && (
        <div role="alert" className="space-y-1">
          <p className="font-medium text-destructive">{a.fix && t.has(key(`fix.${a.fix}`)) ? t(key(`fix.${a.fix}`)) : a.last_error}</p>
          {a.fix && <p className="text-xs break-words text-muted-foreground">{a.last_error}</p>}
        </div>
      )}
      {a.reconnect && oneClick && def.oneClick && <OneClick def={def} account={a.id} />}

      {preview ? (
        <Preview
          account={a}
          def={def}
          pending={hold.isPending || settings.isPending}
          onExclude={(exclude) => settings.mutate({ exclude })}
          onApprove={(h) => hold.mutate(h)}
        />
      ) : (
        <p className="text-muted-foreground tabular-nums">
          {t("stats", {
            items: a.stats.items ?? 0,
            pushed: a.stats.last_push?.pushed ?? 0,
            waiting: a.stats.last_push?.waiting ?? 0,
          })}
        </p>
      )}
      {kept > 0 && <p className="text-muted-foreground">{t("kept", { count: kept, name: def.title })}</p>}

      {a.issues.length > 0 && (
        <details open>
          <summary className="cursor-pointer font-medium">{t("issues", { count: a.issues.length })}</summary>
          <ul className="mt-1 space-y-2">
            {a.issues.map((i) => (
              <li key={i.item} className="text-xs">
                <code className="break-all">{i.item}</code>
                <p className="break-words text-muted-foreground">{i.error}</p>
                {i.fix && t.has(key(`fix.${i.fix}`)) && <p>{t(key(`fix.${i.fix}`))}</p>}
              </li>
            ))}
          </ul>
        </details>
      )}

      <p className="text-xs text-muted-foreground">
        {a.last_webhook_at
          ? t("webhookActive", { when: format.relativeTime(new Date(a.last_webhook_at), now) })
          : t("webhookNone")}
      </p>

      {!preview && (
        <div className="grid gap-2">
          <Toggle
            id={`cn-${a.id}-review`}
            checked={a.hold === "push"}
            disabled={hold.isPending || a.hold === "translate"}
            onChange={(on) => hold.mutate(on ? "push" : null)}
            label={t("reviewFirst")}
          />
          <Toggle
            id={`cn-${a.id}-overwrite`}
            checked={a.settings.overwrite === true}
            disabled={settings.isPending}
            onChange={(on) => settings.mutate({ overwrite: on })}
            label={t("overwrite", { name: def.title })}
          />
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        {a.hold === "push" && (
          <>
            <Button size="sm" onClick={() => publish.mutate()} disabled={publish.isPending || busy}>
              {t("publish")}
            </Button>
            <Button size="sm" variant="outline" onClick={() => toggle("diff")} aria-expanded={panel === "diff"}>
              {t("review")}
            </Button>
          </>
        )}
        <Button size="sm" variant={a.hold ? "outline" : "default"} onClick={() => sync.mutate()} disabled={busy || sync.isPending}>
          {busy ? t("syncing") : t("sync")}
        </Button>
        {a.hold !== "translate" && (
          <Button size="sm" variant="outline" onClick={() => hold.mutate("translate")} disabled={hold.isPending}>
            {t("pause")}
          </Button>
        )}
        <Button size="sm" variant="outline" onClick={() => toggle("doctor")} aria-expanded={panel === "doctor"}>
          {t("doctor.run")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => toggle("history")} aria-expanded={panel === "history"}>
          {t("history.title")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            if (window.confirm(t("removeConfirm", { name: a.name }))) remove.mutate();
          }}
        >
          {t("remove")}
        </Button>
      </div>

      {panel === "doctor" && <DoctorPanel id={a.id} />}
      {panel === "diff" && <DiffPanel id={a.id} />}
      {panel === "history" && <HistoryPanel id={a.id} name={def.title} onChange={done} />}

      <details>
        <summary className="cursor-pointer text-muted-foreground">{t("webhook")}</summary>
        <p className="mt-1 text-xs text-muted-foreground">{t("webhookHint")}</p>
        <code className="mt-1 block break-all rounded bg-muted p-2 text-xs">{a.webhook_url}</code>
      </details>
    </div>
  );
}

function Toggle(p: { id: string; checked: boolean; disabled?: boolean; onChange: (on: boolean) => void; label: string }) {
  return (
    <div className="flex items-center gap-2">
      <Checkbox id={p.id} checked={p.checked} disabled={p.disabled} onCheckedChange={(v) => p.onChange(v === true)} />
      <Label htmlFor={p.id} className="font-normal">
        {p.label}
      </Label>
    </div>
  );
}

/** "412 items, 3 languages, about $1.20": shown before anything is translated or written. */
function Preview(p: {
  account: Connector;
  def: ConnectorDef;
  pending: boolean;
  onExclude: (exclude: string[]) => void;
  onApprove: (hold: Connector["hold"]) => void;
}) {
  const t = useTranslations("connectors");
  const format = useFormatter();
  const s = p.account.stats;
  const excluded = (p.account.settings.exclude as string[] | undefined) ?? [];
  const types = Object.keys(s.by_type ?? {}).sort();
  const included = (ty: string) => !excluded.includes(ty);
  const resources = Object.fromEntries(Object.entries(s.resources ?? {}).filter(([ty]) => included(ty)));
  return (
    <section className="space-y-3 rounded-lg bg-muted/50 p-3" aria-label={t("preview.title")}>
      <h3 className="font-medium">{t("preview.title")}</h3>
      <p className="tabular-nums">
        {t("preview.summary", {
          resources: sum(resources),
          fields: s.items ?? 0,
          languages: s.locales?.length ?? 0,
          words: s.words ?? 0,
        })}
      </p>
      {(s.to_translate ?? 0) > 0 && (
        <p className="text-muted-foreground tabular-nums">
          {t("preview.cost", { cost: format.number(s.cost_usd ?? 0, { style: "currency", currency: "USD" }) })}
        </p>
      )}
      {types.length > 1 && (
        <fieldset className="grid gap-2">
          <legend className="mb-1 text-xs text-muted-foreground">{t("preview.types")}</legend>
          {types.map((ty) => (
            <Toggle
              key={ty}
              id={`cn-${p.account.id}-type-${ty}`}
              checked={included(ty)}
              disabled={p.pending}
              onChange={(on) => p.onExclude(on ? excluded.filter((x) => x !== ty) : [...excluded, ty])}
              label={`${typeLabel(ty)} · ${format.number(s.resources?.[ty] ?? 0)}`}
            />
          ))}
        </fieldset>
      )}
      <p className="text-xs text-muted-foreground">{t("preview.nothing", { name: p.def.title })}</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => p.onApprove(null)} disabled={p.pending}>
          {t("preview.approve")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => p.onApprove("push")} disabled={p.pending}>
          {t("preview.approveReview")}
        </Button>
      </div>
    </section>
  );
}

const MARK: Record<Check["status"], string> = { ok: "✓", warn: "!", fail: "✕" };

function DoctorPanel({ id }: { id: string }) {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const q = useQuery({ queryKey: ["connector-doctor", id], queryFn: () => connectorsApi.doctor(slug, id), staleTime: 0, gcTime: 0 });
  if (q.isPending) return <p role="status" className="text-muted-foreground">{t("doctor.running")}</p>;
  if (q.isError) return <p role="alert" className="text-destructive">{q.error.message}</p>;
  return (
    <section className="space-y-2 rounded-lg border p-3" aria-label={t("doctor.title")}>
      <p className="font-medium">{q.data.ok ? t("doctor.allGood") : t("doctor.problems")}</p>
      <ul className="space-y-2">
        {q.data.checks.map((c, i) => (
          <li key={i} className="flex gap-2">
            <span
              aria-hidden
              className={c.status === "fail" ? "text-destructive" : c.status === "warn" ? "text-amber-600" : "text-emerald-600"}
            >
              {MARK[c.status]}
            </span>
            <div className="min-w-0">
              <p>
                <span className="sr-only">{t(`doctor.status.${c.status}`)}: </span>
                {t.has(key(`doctor.check.${c.id}`)) ? t(key(`doctor.check.${c.id}`)) : typeLabel(c.id)}
                {c.detail && <span className="break-words text-muted-foreground"> · {c.detail}</span>}
              </p>
              {c.fix && t.has(key(`fix.${c.fix}`)) && <p className="text-xs text-muted-foreground">{t(key(`fix.${c.fix}`))}</p>}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function DiffPanel({ id }: { id: string }) {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const q = useQuery({ queryKey: ["connector-diff", id], queryFn: () => connectorsApi.diff(slug, id) });
  if (q.isPending) return <p role="status" className="text-muted-foreground">{t("doctor.running")}</p>;
  if (q.isError) return <p role="alert" className="text-destructive">{q.error.message}</p>;
  if (q.data.total === 0) return <p className="text-muted-foreground">{t("diff.empty")}</p>;
  return (
    <section className="space-y-2 rounded-lg border p-3">
      <p className="font-medium">{t("ready", { count: q.data.total })}</p>
      <ul className="space-y-3">
        {q.data.rows.map((r) => (
          <li key={`${r.locale}/${r.id}/${r.field}`} className="space-y-1 text-xs">
            <p className="flex flex-wrap items-center gap-2">
              <Badge variant={r.blocked ? "destructive" : "secondary"}>
                {r.blocked ? t("diff.blocked") : r.update ? t("diff.update") : t("diff.new")}
              </Badge>
              <span className="text-muted-foreground">
                {r.locale} · {r.field}
              </span>
            </p>
            <p className="line-clamp-3 break-words text-muted-foreground">{r.source}</p>
            <p className="line-clamp-3 break-words" lang={r.locale}>
              {r.value}
            </p>
            {r.fix && t.has(key(`fix.${r.fix}`)) && <p className="text-destructive">{t(key(`fix.${r.fix}`))}</p>}
          </li>
        ))}
      </ul>
      {q.data.total > q.data.rows.length && (
        <p className="text-xs text-muted-foreground">{t("diff.more", { count: q.data.total - q.data.rows.length })}</p>
      )}
    </section>
  );
}

function HistoryPanel({ id, name, onChange }: { id: string; name: string; onChange: () => void }) {
  const t = useTranslations("connectors");
  const format = useFormatter();
  const slug = useSlug();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["connector-runs", id], queryFn: () => connectorsApi.runs(slug, id) });
  const undo = useMutation({
    mutationFn: (run: string) => connectorsApi.rollback(slug, id, run),
    onSuccess: (r) => {
      toast.success(t("history.done", { restored: r.restored, removed: r.removed, kept: r.kept }));
      qc.invalidateQueries({ queryKey: ["connector-runs", id] });
      onChange();
    },
    onError: (e) => toast.error(e.message),
  });
  if (q.isPending) return <p role="status" className="text-muted-foreground">{t("doctor.running")}</p>;
  if (q.isError) return <p role="alert" className="text-destructive">{q.error.message}</p>;
  if (q.data.length === 0) return <p className="text-muted-foreground">{t("history.empty")}</p>;
  // Runs are undone newest first: only the latest one not undone yet has the button.
  const latest = q.data.find((r) => !r.rolled_back_at)?.id;
  return (
    <ul className="space-y-2 rounded-lg border p-3">
      {q.data.map((r) => (
        <li key={r.id} className="flex flex-wrap items-center justify-between gap-2">
          <span className="tabular-nums">
            {format.dateTime(new Date(r.started_at), { dateStyle: "medium", timeStyle: "short" })} ·{" "}
            {t("history.run", { count: r.stats.pushed ?? 0 })}
          </span>
          {r.rolled_back_at ? (
            <Badge variant="secondary">{t("history.rolledBack")}</Badge>
          ) : (
            r.id === latest && (
              <Button
                size="sm"
                variant="outline"
                disabled={undo.isPending}
                onClick={() => {
                  if (window.confirm(t("history.confirm", { name }))) undo.mutate(r.id);
                }}
              >
                {t("history.rollback")}
              </Button>
            )
          )}
        </li>
      ))}
    </ul>
  );
}
