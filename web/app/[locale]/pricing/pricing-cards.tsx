"use client";

import { PlanButton, PlanCards } from "./plans";

/** Public page: CTAs go to sign-up / the billing settings (the login redirect keeps `next`). */
export function PricingCards() {
  return <PlanCards action={(plan) => <PlanButton plan={plan} href={plan === "free" ? "/signup" : "/app/settings/billing"} />} />;
}
