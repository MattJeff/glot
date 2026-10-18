"use client";

import { useState } from "react";
import type { Invoice, InvoiceStatus } from "@/lib/data";
import { InvoiceTable } from "./InvoiceTable";
import { NewInvoiceForm } from "./NewInvoiceForm";
import { EmptyState } from "./EmptyState";

const FILTERS: { value: InvoiceStatus | "all"; label: string }[] = [
  { value: "all", label: "All" },
  { value: "draft", label: "Drafts" },
  { value: "pending", label: "Pending" },
  { value: "overdue", label: "Overdue" },
  { value: "paid", label: "Paid" },
];

export function InvoicesView({ invoices }: { invoices: Invoice[] }) {
  const [filter, setFilter] = useState<InvoiceStatus | "all">("all");
  const [creating, setCreating] = useState(false);
  const shown = filter === "all" ? invoices : invoices.filter((i) => i.status === filter);

  return (
    <div className="space-y-6">
      <div className="flex items-end justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Invoices</h1>
          {/* Trap: plural built by concatenation. */}
          <p className="text-sm text-slate-500">{"Showing " + shown.length + " of " + invoices.length + " invoices"}</p>
        </div>
        <button onClick={() => setCreating(true)} className="w-36 rounded-md bg-brand-600 py-2 text-sm font-medium text-white">
          New invoice
        </button>
      </div>
      {creating && <NewInvoiceForm onClose={() => setCreating(false)} />}
      <div role="tablist" aria-label="Filter by status" className="flex gap-1">
        {FILTERS.map((f) => (
          <button
            key={f.value}
            role="tab"
            aria-selected={filter === f.value}
            onClick={() => setFilter(f.value)}
            className={`rounded-md px-3 py-1.5 text-sm ${filter === f.value ? "bg-slate-900 text-white" : "text-slate-600 hover:bg-slate-100"}`}
          >
            {f.label}
          </button>
        ))}
      </div>
      {shown.length === 0 ? <EmptyState /> : <InvoiceTable invoices={shown} />}
    </div>
  );
}
