"use client";

// Connecting a platform: one click when the server can (OAuth, or the platform's own key authorization),
// the credentials form otherwise or on demand.
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSlug } from "@/lib/hooks";
import { connectorKey, connectorsApi } from "./api";
import type { ConnectorDef } from "./types";

// Per-connector messages (`connectors.<kind>.…`): keys built from CONNECTORS, so not statically typed.
type Key = Parameters<ReturnType<typeof useTranslations<"connectors">>>[0];
export const key = (s: string) => s as Key;

export function ConnectPanel({ def, oneClick, onDone }: { def: ConnectorDef; oneClick: boolean; onDone: () => void }) {
  const t = useTranslations("connectors");
  const [manual, setManual] = useState(false);
  if (!oneClick || !def.oneClick || manual) return <ConnectForm def={def} onDone={onDone} />;
  return (
    <div className="mt-4 grid gap-3 border-t pt-4">
      <OneClick def={def} />
      <div className="flex gap-2">
        <Button type="button" variant="link" size="sm" className="px-0" onClick={() => setManual(true)}>
          {t("manual")}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          {t("cancel")}
        </Button>
      </div>
    </div>
  );
}

/** Leaves for the platform's approval screen; `account` = reconnect that one. */
export function OneClick({ def, account }: { def: ConnectorDef; account?: string }) {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const [tenant, setTenant] = useState("");
  const start = useMutation({
    mutationFn: () => connectorsApi.start(slug, def.kind, { tenant, account }),
    onSuccess: (r) => window.location.assign(r.url),
    onError: (e) => toast.error(e.message),
  });
  const id = `cn-${def.kind}-tenant${account ?? ""}`;
  return (
    <form
      className="grid gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        start.mutate();
      }}
    >
      {def.oneClick?.tenant && (
        <div className="grid gap-1.5">
          <Label htmlFor={id}>{t(key(`tenant.${def.kind}`))}</Label>
          <Input
            id={id}
            required
            autoComplete="off"
            placeholder={def.oneClick.tenant.placeholder}
            value={tenant}
            onChange={(e) => setTenant(e.target.value)}
          />
        </div>
      )}
      <p className="text-xs text-pretty text-muted-foreground">{t("oneClickHint", { name: def.title })}</p>
      <div>
        <Button type="submit" size="sm" disabled={start.isPending || start.isSuccess}>
          {start.isPending || start.isSuccess
            ? t("redirecting", { name: def.title })
            : account
              ? t("reconnect")
              : t("oneClick")}
        </Button>
      </div>
    </form>
  );
}

function ConnectForm({ def, onDone }: { def: ConnectorDef; onDone: () => void }) {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const qc = useQueryClient();
  const [values, setValues] = useState<Record<string, string>>({});
  const create = useMutation({
    mutationFn: () => connectorsApi.create(slug, { kind: def.kind, secrets: values, preview: true }),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: connectorKey(slug) });
      toast.success(t("connectedPreview", { name: r.connector.name }));
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
