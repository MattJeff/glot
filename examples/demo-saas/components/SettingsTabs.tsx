import Link from "next/link";

export function SettingsTabs({ active }: { active: "team" | "billing" }) {
  const tabs = [
    { id: "team", href: "/settings/team", label: "Team" },
    { id: "billing", href: "/settings/billing", label: "Billing" },
  ];
  return (
    <div className="mb-6">
      <h1 className="text-2xl font-semibold">Settings</h1>
      <nav aria-label="Settings sections" className="mt-4 flex gap-4 border-b border-slate-200 text-sm">
        {tabs.map((t) => (
          <Link
            key={t.id}
            href={t.href}
            className={`-mb-px border-b-2 pb-2 ${active === t.id ? "border-brand-600 font-medium text-slate-900" : "border-transparent text-slate-500"}`}
          >
            {t.label}
          </Link>
        ))}
      </nav>
    </div>
  );
}
