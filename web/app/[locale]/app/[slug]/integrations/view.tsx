"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { PageHeader } from "@/components/common";
import { type Connector, connectorKey, connectorsApi } from "@/components/connectors/api";
import { CONNECTORS, type ConnectorDef } from "@/components/connectors";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link } from "@/i18n/navigation";
import { useSlug } from "@/lib/hooks";

// Per-connector messages (`connectors.<kind>.…`): keys built from CONNECTORS, so not statically typed.
type Key = Parameters<ReturnType<typeof useTranslations<"connectors">>>[0];
const key = (s: string) => s as Key;

export function IntegrationsView() {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const list = useQuery({
    queryKey: connectorKey(slug),
    queryFn: () => connectorsApi.list(slug),
    // Live while a sync runs or a translation is pending.
    refetchInterval: (q) => (q.state.data?.some((c) => c.status === "queued" || c.status === "running") ? 3000 : false),
  });
  return (
    <div className="mx-auto max-w-3xl space-y-8">
      <PageHeader title={t("page.title")} subtitle={t("page.subtitle")} />
      <ul className="space-y-4">
        {CONNECTORS.map((def) => (
          <li key={def.kind}>
            <ConnectorCard def={def} accounts={(list.data ?? []).filter((c) => c.kind === def.kind)} />
          </li>
        ))}
      </ul>
      <p className="text-sm text-muted-foreground">
        {t.rich("page.docs", { docs: (c) => <Link href="/docs/integrations" className="underline">{c}</Link> })}
      </p>
    </div>
  );
}

function ConnectorCard({ def, accounts }: { def: ConnectorDef; accounts: Connector[] }) {
  const t = useTranslations("connectors");
  const [open, setOpen] = useState(false);
  return (
    <section aria-labelledby={`cn-${def.kind}`} className="rounded-xl border p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id={`cn-${def.kind}`} className="font-medium">
            {def.title}
          </h2>
          <p className="text-sm text-pretty text-muted-foreground">{t(key(`${def.kind}.description`))}</p>
        </div>
        <Button variant={accounts.length ? "outline" : "default"} size="sm" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
          {t("connect")}
        </Button>
      </div>
      {accounts.map((a) => (
        <AccountRow key={a.id} account={a} />
      ))}
      {open && <ConnectForm def={def} onDone={() => setOpen(false)} />}
    </section>
  );
}

function ConnectForm({ def, onDone }: { def: ConnectorDef; onDone: () => void }) {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const qc = useQueryClient();
  const [values, setValues] = useState<Record<string, string>>({});
  const create = useMutation({
    mutationFn: () => connectorsApi.create(slug, { kind: def.kind, secrets: values }),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: connectorKey(slug) });
      toast.success(t("connected", { name: r.connector.name }));
      if (r.source_locale_mismatch) toast.warning(t("localeMismatch", { locale: r.info.source_locale ?? "" }));
      if (r.webhooks_error) toast.info(t("webhooksError"));
      onDone();
    },
    onError: (e) => toast.error(e.message),
  });
  return (
    <form
      className="mt-4 grid gap-3 border-t pt-4"
      onSubmit={(e) => {
        e.preventDefault();
        create.mutate();
      }}
    >
      {def.fields.map((f) => {
        const id = `cn-${def.kind}-${f.name}`;
        return (
          <div key={f.name} className="grid gap-1.5">
            <Label htmlFor={id}>
              {t(key(`${def.kind}.fields.${f.name}`))}
              {f.optional && <span className="font-normal text-muted-foreground"> ({t("optional")})</span>}
            </Label>
            <Input
              id={id}
              type={f.secret ? "password" : (f.type ?? "text")}
              autoComplete={f.secret ? "new-password" : "off"}
              placeholder={f.placeholder}
              required={!f.optional}
              value={values[f.name] ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, [f.name]: e.target.value }))}
            />
            {t.has(key(`${def.kind}.hints.${f.name}`)) && (
              <p className="text-xs text-muted-foreground">{t(key(`${def.kind}.hints.${f.name}`))}</p>
            )}
          </div>
        );
      })}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={create.isPending}>
          {create.isPending ? t("testing") : t("save")}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          {t("cancel")}
        </Button>
      </div>
    </form>
  );
}

function AccountRow({ account: a }: { account: Connector }) {
  const t = useTranslations("connectors");
  const format = useFormatter();
  const now = useNow({ updateInterval: 30_000 });
  const slug = useSlug();
  const qc = useQueryClient();
  const done = () => qc.invalidateQueries({ queryKey: connectorKey(slug) });
  const sync = useMutation({
    mutationFn: () => connectorsApi.sync(slug, a.id),
    onSuccess: () => {
      done();
      toast.success(t("syncQueued"));
    },
    onError: (e) => toast.error(e.message),
  });
  const test = useMutation({
    mutationFn: () => connectorsApi.test(slug, a.id),
    onSuccess: (r) => toast.success(t("testOk", { name: r.info.name, locales: r.locales.join(", ") })),
    onError: (e) => toast.error(e.message),
  });
  const remove = useMutation({
    mutationFn: () => connectorsApi.remove(slug, a.id),
    onSuccess: done,
    onError: (e) => toast.error(e.message),
  });
  const busy = a.status === "queued" || a.status === "running";
  return (
    <div className="mt-4 space-y-2 border-t pt-4 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{a.name}</span>
        <Badge variant={a.status === "error" ? "destructive" : "secondary"}>{t(`status.${a.status}`)}</Badge>
        {a.last_sync_at && (
          <span className="text-muted-foreground">
            {t("lastSync", { when: format.relativeTime(new Date(a.last_sync_at), now) })}
          </span>
        )}
      </div>
      <p className="text-muted-foreground tabular-nums">
        {t("stats", {
          items: a.stats.items ?? 0,
          pushed: a.stats.last_push?.pushed ?? 0,
          waiting: a.stats.last_push?.waiting ?? 0,
        })}
      </p>
      {a.last_error && (
        <p role="alert" className="text-destructive break-words">
          {a.last_error}
        </p>
      )}
      <details>
        <summary className="cursor-pointer text-muted-foreground">{t("webhook")}</summary>
        <p className="mt-1 text-xs text-muted-foreground">{t("webhookHint")}</p>
        <code className="mt-1 block break-all rounded bg-muted p-2 text-xs">{a.webhook_url}</code>
      </details>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => sync.mutate()} disabled={busy || sync.isPending}>
          {busy ? t("syncing") : t("sync")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => test.mutate()} disabled={test.isPending}>
          {t("test")}
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
    </div>
  );
}
