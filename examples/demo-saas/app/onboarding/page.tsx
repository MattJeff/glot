import { EmptyState } from "@/components/EmptyState";
import { OnboardingChecklist } from "@/components/OnboardingChecklist";

export default function OnboardingPage() {
  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Welcome to Acme Invoices 👋</h1>
        <p className="text-sm text-slate-500">Let&apos;s get your workspace ready. This only takes a few minutes.</p>
      </div>
      <OnboardingChecklist />
      <EmptyState />
    </div>
  );
}
