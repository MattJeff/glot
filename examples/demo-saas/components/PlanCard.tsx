import { PLANS } from "@/lib/data";

export function PlanCard({ plan, yearly }: { plan: (typeof PLANS)[number]; yearly: boolean }) {
  const price = yearly ? Math.round(plan.price * 10) : plan.price;
  return (
    <div className={`rounded-2xl border bg-white p-6 ${plan.highlighted ? "border-brand-500 ring-2 ring-brand-500" : "border-slate-200"}`}>
      {plan.highlighted && <p className="mb-2 text-xs font-semibold uppercase text-brand-600">Most popular</p>}
      <h3 className="text-lg font-semibold">{plan.name}</h3>
      <p className="mt-1 text-sm text-slate-500">{plan.tagline}</p>
      <p className="mt-4">
        <span className="text-4xl font-bold">${price}</span>
        <span className="text-sm text-slate-500">{yearly ? " / year" : " / month"}</span>
      </p>
      <ul className="mt-6 space-y-2 text-sm">
        {plan.features.map((f) => (
          <li key={f} className="flex gap-2">
            <span className="text-emerald-600">✓</span>
            {f}
          </li>
        ))}
      </ul>
      <button className={`mt-6 w-full rounded-md py-2 text-sm font-medium ${plan.highlighted ? "bg-brand-600 text-white" : "border border-slate-200"}`}>
        {plan.cta}
      </button>
    </div>
  );
}
