"use client";

import type { PlansInfo } from "@/lib/api";
import { PlanButton, PlanCards } from "./plans";

/** Public page: CTAs go to sign-up / the billing settings (the login redirect keeps `next`). */
export function PricingCards({ initial }: { initial?: PlansInfo }) {
  return <PlanCards initial={initial} action={(plan) => <PlanButton plan={plan} href={plan === "free" ? "/signup" : "/app/settings/billing"} />} />;
}
