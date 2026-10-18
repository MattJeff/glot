import Link from "next/link";
import type { Invoice } from "@/lib/data";
import { formatDate, formatMoney } from "@/lib/format";
import { StatusBadge } from "./StatusBadge";

export function InvoiceTable({ invoices }: { invoices: Invoice[] }) {
  return (
    <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
      <table className="w-full text-left text-sm">
        <caption className="sr-only">List of invoices</caption>
        <thead className="bg-slate-50 text-xs uppercase tracking-wide text-slate-500">
          <tr>
            <th className="px-4 py-3">Invoice</th>
            <th className="px-4 py-3">Customer</th>
            <th className="px-4 py-3">Issued</th>
            <th className="px-4 py-3">Due date</th>
            <th className="px-4 py-3">Status</th>
            <th className="px-4 py-3 text-right">Amount</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-slate-100">
          {invoices.map((inv) => (
            <tr key={inv.id} className="hover:bg-slate-50">
              <td className="px-4 py-3 font-medium">
                <Link href={`/invoices/${inv.id}`} className="text-brand-600 hover:underline">
                  {inv.number}
                </Link>
              </td>
              <td className="px-4 py-3">{inv.customer}</td>
              <td className="px-4 py-3 text-slate-500">{formatDate(inv.issuedAt)}</td>
              <td className="px-4 py-3 text-slate-500">{formatDate(inv.dueAt)}</td>
              <td className="px-4 py-3">
                <StatusBadge status={inv.status} />
              </td>
              <td className="px-4 py-3 text-right tabular-nums">{formatMoney(inv.amount, inv.currency)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
