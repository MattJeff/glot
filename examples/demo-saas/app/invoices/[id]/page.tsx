import Link from "next/link";
import { notFound } from "next/navigation";
import { invoices } from "@/lib/data";
import { daysUntil, formatDate, formatMoney, pluralize } from "@/lib/format";
import { StatusBadge } from "@/components/StatusBadge";
import { InvoiceActions } from "@/components/InvoiceActions";

export function generateStaticParams() {
  return invoices.map((i) => ({ id: i.id }));
}

export default async function InvoiceDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const invoice = invoices.find((i) => i.id === id);
  if (!invoice) notFound();
  const subtotal = invoice.items.reduce((s, it) => s + it.quantity * it.unitPrice, 0);

  return (
    <div className="space-y-6">
      <Link href="/invoices" className="text-sm text-slate-500 hover:text-slate-900">← Back to invoices</Link>
      <div className="flex items-start justify-between">
        <div>
          <h1 className="flex items-center gap-3 text-2xl font-semibold">
            Invoice {invoice.number} <StatusBadge status={invoice.status} />
          </h1>
          <p className="mt-1 text-sm text-slate-500">
            Billed to {invoice.customer} · {daysUntil(invoice.dueAt)}
          </p>
        </div>
        <InvoiceActions number={invoice.number} customer={invoice.customer} />
      </div>

      <div className="grid gap-6 rounded-xl border border-slate-200 bg-white p-6 md:grid-cols-3">
        <div>
          <p className="text-xs uppercase text-slate-500">Bill to</p>
          <p className="mt-1 font-medium">{invoice.customer}</p>
          <p className="text-sm text-slate-500">{invoice.email}</p>
        </div>
        <div>
          <p className="text-xs uppercase text-slate-500">Issue date</p>
          <p className="mt-1">{formatDate(invoice.issuedAt)}</p>
        </div>
        <div>
          <p className="text-xs uppercase text-slate-500">Due date</p>
          <p className="mt-1">{formatDate(invoice.dueAt)}</p>
        </div>
      </div>

      <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
        <table className="w-full text-sm">
          <thead className="bg-slate-50 text-left text-xs uppercase text-slate-500">
            <tr>
              <th className="px-4 py-3">Description</th>
              <th className="px-4 py-3 text-right">Qty</th>
              <th className="px-4 py-3 text-right">Unit price</th>
              <th className="px-4 py-3 text-right">Total</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-100">
            {invoice.items.map((it) => (
              <tr key={it.description}>
                <td className="px-4 py-3">{it.description}</td>
                <td className="px-4 py-3 text-right">{it.quantity}</td>
                <td className="px-4 py-3 text-right">{formatMoney(it.unitPrice, invoice.currency)}</td>
                <td className="px-4 py-3 text-right">{formatMoney(it.quantity * it.unitPrice, invoice.currency)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="space-y-1 border-t border-slate-200 p-4 text-right text-sm">
          <p className="text-slate-500">{pluralize(invoice.items.length, "item")}</p>
          <p>Subtotal: {formatMoney(subtotal, invoice.currency)}</p>
          <p className="text-slate-500">Tax (0%): {formatMoney(0, invoice.currency)}</p>
          <p className="text-lg font-semibold">Amount due: {formatMoney(invoice.amount, invoice.currency)}</p>
        </div>
      </div>
    </div>
  );
}
