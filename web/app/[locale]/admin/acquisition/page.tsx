"use client";

import { useQuery } from "@tanstack/react-query";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { ErrorState, Logo, ThemeToggle } from "@/components/common";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";

type Row = { source: string | null; medium: string | null; campaign: string | null; term: string | null; signups: number; verified: number; paying: number };

/** Instance admins: what each ad campaign brought (signups, confirmed emails, paying accounts). */
export default function AcquisitionPage() {
  const t = useTranslations("adminAcquisition");
  const [days, setDays] = useState(30);
  const q = useQuery({
    queryKey: ["admin-acquisition", days],
    queryFn: async () => {
      const r = await fetch(`/api/admin/acquisition?days=${days}`, { credentials: "include" });
      if (!r.ok) throw new Error(String(r.status));
      return (await r.json()) as { rows: Row[] };
    },
  });
  const dash = (v: string | null) => v ?? "—";
  return (
    <div className="app-canvas min-h-screen">
      <header className="flex h-16 items-center justify-between px-6">
        <Link href="/app"><Logo className="text-xl" /></Link>
        <ThemeToggle />
      </header>
      <main className="mx-auto max-w-5xl px-6 pb-16">
        <h1 className="text-3xl font-bold tracking-tight">{t("title")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("subtitle")}</p>
        <div role="radiogroup" aria-label={t("period")} className="mt-6 flex w-fit rounded-full border p-0.5 text-sm">
          {[7, 30, 90].map((d) => (
            <button key={d} type="button" role="radio" aria-checked={days === d} onClick={() => setDays(d)} className={`rounded-full px-3 py-1 ${days === d ? "bg-primary/15 font-semibold" : "text-muted-foreground"}`}>
              {t("days", { n: d })}
            </button>
          ))}
        </div>
        {q.isPending && <Skeleton className="mt-6 h-40 rounded-2xl" />}
        {q.isError && <div className="mt-6"><ErrorState error={q.error} onRetry={() => q.refetch()} /></div>}
        {q.data && (
          <div className="mt-6 overflow-x-auto rounded-2xl border bg-card">
            <table className="w-full text-sm">
              <thead className="text-start text-muted-foreground">
                <tr>{(["source", "medium", "campaign", "term", "signups", "verified", "paying"] as const).map((c) => <th key={c} className="px-4 py-3 text-start font-medium">{t(`cols.${c}`)}</th>)}</tr>
              </thead>
              <tbody>
                {q.data.rows.map((r, i) => (
                  <tr key={i} className="border-t">
                    <td className="px-4 py-2 font-medium">{r.source ?? t("direct")}</td>
                    <td className="px-4 py-2">{dash(r.medium)}</td>
                    <td className="px-4 py-2">{dash(r.campaign)}</td>
                    <td className="px-4 py-2">{dash(r.term)}</td>
                    <td className="px-4 py-2 tabular-nums">{r.signups}</td>
                    <td className="px-4 py-2 tabular-nums">{r.verified}</td>
                    <td className="px-4 py-2 font-semibold tabular-nums">{r.paying}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </main>
    </div>
  );
}
