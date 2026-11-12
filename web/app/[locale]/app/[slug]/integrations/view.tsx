"use client";

import { useQuery } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { PageHeader } from "@/components/common";
import { AccountRow } from "@/components/connectors/account";
import { type Connector, connectorKey, connectorsApi } from "@/components/connectors/api";
import { ConnectPanel, key } from "@/components/connectors/connect";
import { CONNECTORS, type ConnectorDef } from "@/components/connectors";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { useSlug } from "@/lib/hooks";

export function IntegrationsView() {
  const t = useTranslations("connectors");
  const slug = useSlug();
  const list = useQuery({
    queryKey: connectorKey(slug),
    queryFn: () => connectorsApi.list(slug),
    // Live while a sync runs or a translation is in progress.
    refetchInterval: (q) =>
      q.state.data?.some((c) => c.status === "queued" || c.status === "running" || c.job) ? 3000 : false,
  });
  // Kinds the server can connect in one click; the others (and all of them on error) keep the manual form.
  const oneClick = useQuery({ queryKey: ["connectors-one-click", slug], queryFn: () => connectorsApi.oneClick(slug), staleTime: Infinity });
  // Back from the platform's approval screen (`?connected=<kind>` or `?connect_error=<message>`).
  useEffect(() => {
    const q = new URLSearchParams(window.location.search);
    const [kind, error] = [q.get("connected"), q.get("connect_error")];
    if (!kind && !error) return;
    if (error) toast.error(t("connectError", { error }));
    else toast.success(t("connectedPreview", { name: CONNECTORS.find((c) => c.kind === kind)?.title ?? kind ?? "" }));
    window.history.replaceState(null, "", window.location.pathname);
  }, [t]);
  return (
    <div className="mx-auto max-w-3xl space-y-8">
      <PageHeader title={t("page.title")} subtitle={t("page.subtitle")} />
      <ul className="space-y-4">
        {CONNECTORS.map((def) => (
          <li key={def.kind}>
            <ConnectorCard
              def={def}
              accounts={(list.data ?? []).filter((c) => c.kind === def.kind)}
              oneClick={oneClick.data?.includes(def.kind) ?? false}
            />
          </li>
        ))}
      </ul>
      <p className="text-sm text-muted-foreground">
        {t.rich("page.docs", { docs: (c) => <Link href="/docs/integrations" className="underline">{c}</Link> })}
      </p>
    </div>
  );
}

function ConnectorCard({ def, accounts, oneClick }: { def: ConnectorDef; accounts: Connector[]; oneClick: boolean }) {
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
        <AccountRow key={a.id} account={a} def={def} oneClick={oneClick} />
      ))}
      {open && <ConnectPanel def={def} oneClick={oneClick} onDone={() => setOpen(false)} />}
    </section>
  );
}
