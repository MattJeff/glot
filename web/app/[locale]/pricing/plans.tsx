"use client";

// Plan cards shared by /pricing and /app/settings/billing. Limits come from the server config and prices from
// Stripe (GET /api/billing/plans), so changing either never needs a web deploy.
import { useQuery } from "@tanstack/react-query";
import { Check } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { api, type Limits, type PlanId, type PlansInfo, type QuotaKey } from "@/lib/api";
import { cn } from "@/lib/utils";

export const PLANS: PlanId[] = ["free", "pro", "team"];
export const QUOTAS: QuotaKey[] = ["projects", "seats", "cdn_requests", "insights_events", "mcp_calls", "ondemand_jobs", "llm_strings"];
export type Interval = "month" | "year";

export function usePlans() {
  return useQuery({ queryKey: ["billing-plans"], queryFn: api.billingPlans, staleTime: 5 * 60_000 });
}

export function useQuotaLabel() {
  const t = useTranslations("pricing.quota");
  const format = useFormatter();
  return (key: QuotaKey, n: number | null | undefined) =>
    n === null || n === undefined
      ? t("unlimited", { name: t(key) })
      : n === 0
        ? t("notIncluded", { name: t(key) })
        : t("value", { name: t(key), n: format.number(n) });
}

function money(format: ReturnType<typeof useFormatter>, cents: number, currency: string) {
  return format.number(cents / 100, { style: "currency", currency: currency.toUpperCase(), maximumFractionDigits: cents % 100 ? 2 : 0 });
}

export function PlanCards({
  current,
  action,
}: {
  /** Plan in force for the signed-in account (billing page). */
  current?: string;
  /** Button for a plan; receives the selected billing interval. */
  action: (plan: PlanId, interval: Interval) => React.ReactNode;
}) {
  const t = useTranslations("pricing");
  const format = useFormatter();
  const label = useQuotaLabel();
  const plans = usePlans();
  const [interval, setInterval] = useState<Interval>("month");
  const info: PlansInfo | undefined = plans.data;
  const llm = info?.prices.glot_llm_strings;

  const price = (plan: PlanId) => {
    if (plan === "free") return { main: money(format, 0, info?.prices.glot_pro_month?.currency ?? "eur"), per: t("forever") };
    const p = info?.prices[`glot_${plan}_${interval}`];
    if (!p?.unit_amount) return { main: "—", per: t("perMonth") };
    return { main: money(format, p.unit_amount, p.currency), per: interval === "month" ? t("perMonth") : t("perYear") };
  };

  return (
    <div>
      <div role="radiogroup" aria-label={t("interval")} className="mx-auto mb-6 flex w-fit rounded-lg border p-0.5 text-sm">
        {(["month", "year"] as const).map((i) => (
          <button
            key={i}
            type="button"
            role="radio"
            aria-checked={interval === i}
            onClick={() => setInterval(i)}
            className={cn("rounded-md px-3 py-1", interval === i ? "bg-accent font-medium" : "text-muted-foreground")}
          >
            {i === "month" ? t("monthly") : t("yearly")}
          </button>
        ))}
      </div>
      <div className="grid gap-4 md:grid-cols-3">
        {PLANS.map((plan) => {
          const limits: Limits = info?.plans[plan] ?? {};
          const { main, per } = price(plan);
          return (
            <section
              key={plan}
              aria-labelledby={`plan-${plan}`}
              className={cn("flex flex-col rounded-xl border p-5", plan === "pro" && "border-primary shadow-sm")}
            >
              <div className="flex items-center gap-2">
                <h3 id={`plan-${plan}`} className="text-base font-semibold">{t(`plans.${plan}.name`)}</h3>
                {current === plan && <Badge variant="secondary">{t("current")}</Badge>}
              </div>
              <p className="mt-1 text-sm text-muted-foreground">{t(`plans.${plan}.tagline`)}</p>
              <p className="mt-4">
                <span className="text-3xl font-semibold tracking-tight">{main}</span>{" "}
                <span className="text-sm text-muted-foreground">{per}</span>
              </p>
              <ul className="mt-4 flex-1 space-y-1.5 text-sm">
                <li className="flex gap-2"><Check className="mt-0.5 size-4 shrink-0 text-success" />{t("ossIncluded")}</li>
                {plans.isPending
                  ? null
                  : QUOTAS.map((q) => (
                      <li key={q} className="flex gap-2">
                        <Check className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                        {label(q, limits[q])}
                      </li>
                    ))}
              </ul>
              <div className="mt-5">{action(plan, interval)}</div>
            </section>
          );
        })}
      </div>
      {llm?.unit_amount_decimal && (
        <p className="mt-4 text-center text-sm text-muted-foreground">
          {t("usage", {
            price: money(format, Number(llm.unit_amount_decimal) * 1000, llm.currency),
          })}
        </p>
      )}
      {info && !info.enabled && <p className="mt-4 text-center text-sm text-muted-foreground">{t("selfHosted")}</p>}
    </div>
  );
}

export function PlanButton({
  plan,
  current,
  href,
  ...props
}: { plan: PlanId; current?: string; href?: string } & React.ComponentProps<typeof Button>) {
  const t = useTranslations("pricing");
  const label = current === plan ? t("current") : plan === "free" ? t("cta.free") : t("cta.upgrade", { plan: t(`plans.${plan}.name`) });
  const variant = plan === "pro" ? "default" : "outline";
  if (href)
    return (
      <Button className="w-full" variant={variant} asChild>
        <Link href={href}>{label}</Link>
      </Button>
    );
  return (
    <Button className="w-full" variant={variant} disabled={current === plan} {...props}>
      {label}
    </Button>
  );
}
