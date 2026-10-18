import Link from "next/link";
import { invoices } from "@/lib/data";
import { formatMoney, pluralize } from "@/lib/format";
import { StatCard } from "@/components/StatCard";
import { InvoiceTable } from "@/components/InvoiceTable";
import { ActivityFeed } from "@/components/ActivityFeed";

export default function DashboardPage() {
  const overdue = invoices.filter((i) => i.status === "overdue");
  const outstanding = invoices.filter((i) => i.status !== "paid").reduce((s, i) => s + i.amount, 0);

  return (
    <div className="space-y-8">
      <div className="flex items-end justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Good morning, Ada</h1>
          <p className="text-sm text-slate-500">Here&apos;s what&apos;s happening with your invoices today.</p>
        </div>
        <Link href="/invoices" className="w-36 rounded-md bg-brand-600 py-2 text-center text-sm font-medium text-white">
          New invoice
        </Link>
      </div>

      {overdue.length > 0 && (
        <div role="status" className="rounded-lg border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-700">
          {"You have " + overdue.length + " overdue invoices"} — <Link href="/invoices" className="font-medium underline">review them now</Link>
        </div>
      )}

      <div className="grid gap-4 md:grid-cols-4">
        <StatCard label="Outstanding" value={formatMoney(outstanding)} delta="+12% vs last month" />
        <StatCard label="Paid this month" value={formatMoney(1200)} delta="+4% vs last month" />
        <StatCard label="Average time to pay" value="9 days" delta="2 days faster" />
        <StatCard label="Customers" value={pluralize(invoices.length, "customer")} delta="1 new this week" />
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="space-y-3 lg:col-span-2">
          <div className="flex items-center justify-between">
            <h2 className="font-semibold">Latest invoices</h2>
            <Link href="/invoices" className="text-sm text-brand-600">View all</Link>
          </div>
          <InvoiceTable invoices={invoices.slice(0, 4)} />
        </div>
        <ActivityFeed />
      </div>
    </div>
  );
}
