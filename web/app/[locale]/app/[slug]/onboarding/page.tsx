"use client";

import { OnboardingChecklist } from "@/components/onboarding/checklist";
import { useSlug } from "@/lib/hooks";

export default function OnboardingPage() {
  return <OnboardingChecklist slug={useSlug()} />;
}
