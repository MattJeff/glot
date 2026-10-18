"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { CreditCard, ExternalLink, Loader2 } from "lucide-react";
import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { ErrorState, PageHeader } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { api, qk } from "@/lib/api";
import { PlanButton, PlanCards, QUOTAS, useQuotaLabel } from "../../../pricing/plans";

function BillingContent() {
  const t = useTranslations("billing");
  const tp = useTranslations("pricing");
  const format = useFormatter();
  const label = useQuotaLabel();
  const search = useSearchParams();
  const billing = useQuery({ queryKey: qk.billing, queryFn: api.billing });
  const enabled = billing.data?.enabled === true;
  const invoices = useQuery({ queryKey: qk.invoices, queryFn: api.invoices, enabled: enabled && billing.data?.has_customer });
  const go = ({ url }: { url: string }) => window.location.assign(url);
  const checkout = useMutation({ mutationFn: api.checkout, onSuccess: go });
  const portal = useMutation({ mutationFn: api.billingPortal, onSuccess: go });

  if (billing.isPending) return <Skeleton className="mx-auto h-64 max-w-4xl rounded-xl" />;
  if (billing.isError) return <ErrorState error={billing.error} onRetry={() => billing.refetch()} />;
  const b = billing.data;
  const date = (d: string) => format.dateTime(new Date(d), { dateStyle: "medium" });
  const notice = search.get("checkout") === "success" || search.get("changed") ? t("updated") : null;

  if (!b.enabled)
    return (
      <div className="mx-auto max-w-4xl">
        <PageHeader title={t("title")} subtitle={t("selfHosted")} />
      </div>
    );

  return (
    <div className="mx-auto max-w-4xl space-y-8">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          b.has_customer && (
            <Button variant="outline" onClick={() => portal.mutate()} disabled={portal.isPending}>
              {portal.isPending ? <Loader2 className="animate-spin" /> : <CreditCard />} {t("manage")}
            </Button>
          )
        }
      />
      {notice && <p role="status" className="rounded-md border bg-muted/40 px-3 py-2 text-sm">{notice}</p>}
      {b.grace_until && b.status === "past_due" && (
        <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm">
          {t("pastDue", { date: date(b.grace_until) })}
        </p>
      )}

      <section className="rounded-xl border p-5">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h2 className="text-base font-semibold">{t("plan", { plan: tp(`plans.${b.plan as "free"}.name`) })}</h2>
          {b.current_period_end && b.plan !== "free" && (
            <span className="text-sm text-muted-foreground">
              {b.cancel_at_period_end ? t("endsOn", { date: date(b.current_period_end) }) : t("renewsOn", { date: date(b.current_period_end) })}
            </span>
          )}
        </div>
        <h3 className="mt-5 mb-3 text-sm font-medium text-muted-foreground">{t("usage")}</h3>
        <ul className="grid gap-4 sm:grid-cols-2">
          {QUOTAS.filter((q) => q !== "seats").map((q) => {
            const used = b.usage[q] ?? 0;
            const max = b.limits[q];
            const pct = max ? Math.min(100, (used / max) * 100) : 0;
            return (
              <li key={q}>
                <div className="flex justify-between gap-2 text-sm">
                  <span>{label(q, max)}</span>
                  <span className="text-muted-foreground tabular-nums">{format.number(used)}</span>
                </div>
                <Progress value={pct} className={pct >= 100 ? "[&>*]:bg-destructive" : pct >= 80 ? "[&>*]:bg-amber-500" : ""} />
              </li>
            );
          })}
        </ul>
        {b.metered && <p className="mt-4 text-xs text-muted-foreground">{t("metered")}</p>}
        <p className="mt-4 text-xs text-muted-foreground">{t("ossNote")}</p>
      </section>

      <PlanCards
        current={b.plan}
        action={(plan, interval) =>
          plan === "free" ? (
            b.plan !== "free" && b.has_customer ? (
              <Button variant="outline" className="w-full" onClick={() => portal.mutate()}>{t("downgrade")}</Button>
            ) : (
              <PlanButton plan={plan} current={b.plan} />
            )
          ) : (
            <PlanButton
              plan={plan}
              current={b.plan}
              disabled={b.plan === plan || checkout.isPending}
              onClick={() => checkout.mutate({ plan, interval })}
            />
          )
        }
      />

      {b.has_customer && (
        <section>
          <h2 className="mb-3 text-base font-semibold">{t("invoices")}</h2>
          {invoices.isPending ? (
            <Skeleton className="h-24 rounded-xl" />
          ) : invoices.isError ? (
            <ErrorState error={invoices.error} onRetry={() => invoices.refetch()} />
          ) : invoices.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("noInvoices")}</p>
          ) : (
            <div className="overflow-x-auto rounded-xl border">
              <table className="w-full text-sm">
                <thead className="border-b bg-muted/40 text-xs text-muted-foreground">
                  <tr>
                    <th scope="col" className="px-4 py-2 text-start font-medium">{t("invoice.date")}</th>
                    <th scope="col" className="px-4 py-2 text-start font-medium">{t("invoice.number")}</th>
                    <th scope="col" className="px-4 py-2 text-start font-medium">{t("invoice.total")}</th>
                    <th scope="col" className="px-4 py-2 text-start font-medium">{t("invoice.status")}</th>
                    <th scope="col"><span className="sr-only">{t("invoice.open")}</span></th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {invoices.data.map((inv) => (
                    <tr key={inv.id}>
                      <td className="px-4 py-2.5">{inv.created ? date(inv.created) : "—"}</td>
                      <td className="px-4 py-2.5 font-mono text-xs">{inv.number ?? "—"}</td>
                      <td className="px-4 py-2.5 tabular-nums">
                        {format.number(inv.total / 100, { style: "currency", currency: inv.currency.toUpperCase() })}
                      </td>
                      <td className="px-4 py-2.5 text-muted-foreground">{inv.status}</td>
                      <td className="px-2 text-end">
                        {(inv.hosted_invoice_url ?? inv.invoice_pdf) && (
                          <Button variant="ghost" size="sm" asChild>
                            <a href={inv.hosted_invoice_url ?? inv.invoice_pdf ?? ""} target="_blank" rel="noreferrer">
                              {t("invoice.open")} <ExternalLink />
                            </a>
                          </Button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      )}
    </div>
  );
}

// useSearchParams() needs a Suspense boundary for static prerendering (Next 16).
export default function BillingPage() {
  return (
    <Suspense>
      <BillingContent />
    </Suspense>
  );
}
