import Link from "next/link";
import { NAV } from "@/lib/data";

export function Sidebar() {
  return (
    <aside className="hidden w-60 shrink-0 border-r border-slate-200 bg-white p-4 md:block">
      <Link href="/" className="mb-8 flex items-center gap-2 px-2 text-lg font-semibold">
        <span className="grid h-8 w-8 place-items-center rounded-lg bg-brand-600 text-white">A</span>
        Acme Invoices
      </Link>
      <nav aria-label="Main navigation" className="space-y-1">
        {NAV.map((item) => (
          <Link
            key={item.href}
            href={item.href}
            className="flex items-center gap-3 rounded-md px-2 py-2 text-sm text-slate-600 hover:bg-slate-100 hover:text-slate-900"
          >
            <span aria-hidden>{item.icon}</span>
            {item.label}
          </Link>
        ))}
      </nav>
      <div className="mt-8 rounded-lg bg-brand-50 p-3 text-sm">
        <p className="font-medium text-brand-700">Trial ends in 6 days</p>
        <p className="mt-1 text-slate-600">Upgrade to keep sending reminders.</p>
        <Link href="/pricing" className="mt-2 inline-block font-medium text-brand-600">
          See plans →
        </Link>
      </div>
    </aside>
  );
}
