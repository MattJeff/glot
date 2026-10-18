import Link from "next/link";
import { formatDate, formatMoney } from "@/lib/format";
import { SettingsTabs } from "@/components/SettingsTabs";

const HISTORY = [
  { date: "2026-10-01", description: "Growth plan — monthly", amount: 29 },
  { date: "2026-09-01", description: "Growth plan — monthly", amount: 29 },
  { date: "2026-08-01", description: "Growth plan — monthly", amount: 29 },
];

export default function BillingSettingsPage() {
  return (
    <div>
      <SettingsTabs active="billing" />
      <div className="grid gap-6 md:grid-cols-2">
        <section className="rounded-xl border border-slate-200 bg-white p-6">
          <h2 className="font-semibold">Current plan</h2>
          <p className="mt-2 text-3xl font-bold">Growth</p>
          <p className="text-sm text-slate-500">{formatMoney(29)} per month, renews on {formatDate("2026-11-01")}</p>
          <div className="mt-4 flex gap-2">
            <Link href="/pricing" className="w-32 rounded-md bg-brand-600 py-2 text-center text-sm font-medium text-white">Change plan</Link>
            <button className="w-32 rounded-md border border-slate-200 py-2 text-sm">Cancel plan</button>
          </div>
        </section>
        <section className="rounded-xl border border-slate-200 bg-white p-6">
          <h2 className="font-semibold">Payment method</h2>
          <p className="mt-2 text-sm">Visa ending in 4242</p>
          <p className="text-sm text-slate-500">Expires 08/2028</p>
          <button className="mt-4 text-sm font-medium text-brand-600">Update card</button>
        </section>
      </div>
      <section className="mt-6 rounded-xl border border-slate-200 bg-white p-6">
        <h2 className="font-semibold">Billing history</h2>
        <table className="mt-4 w-full text-sm">
          <thead className="text-left text-xs uppercase text-slate-500">
            <tr>
              <th className="py-2">Date</th>
              <th className="py-2">Description</th>
              <th className="py-2 text-right">Amount</th>
              <th className="py-2 text-right"><span className="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-100">
            {HISTORY.map((h) => (
              <tr key={h.date}>
                <td className="py-2">{formatDate(h.date)}</td>
                <td className="py-2">{h.description}</td>
                <td className="py-2 text-right">{formatMoney(h.amount)}</td>
                <td className="py-2 text-right">
                  <a href="#" className="text-brand-600">Download PDF</a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}
