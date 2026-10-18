"use client";

import { useState } from "react";
import { useToast } from "./Toast";

export function NewInvoiceForm({ onClose }: { onClose: () => void }) {
  const toast = useToast();
  const [lines, setLines] = useState(1);
  const [error, setError] = useState("");

  function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    if (!data.get("customer")) {
      setError("Please choose a customer before saving.");
      return;
    }
    toast(`Draft created with ${lines} line items`);
    onClose();
  }

  return (
    <form onSubmit={submit} aria-label="New invoice" className="space-y-4 rounded-xl border border-slate-200 bg-white p-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">New invoice</h2>
        <button type="button" onClick={onClose} aria-label="Close form" className="text-slate-400 hover:text-slate-900">
          ✕
        </button>
      </div>
      <div className="grid grid-cols-2 gap-4">
        <label className="text-sm">
          <span className="font-medium">Customer</span>
          <input name="customer" placeholder="e.g. Globex Corporation" className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2" />
        </label>
        <label className="text-sm">
          <span className="font-medium">Billing email</span>
          <input name="email" type="email" placeholder="billing@company.com" className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2" />
        </label>
        <label className="text-sm">
          <span className="font-medium">Due date</span>
          <input name="due" type="date" className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2" />
        </label>
        <label className="text-sm">
          <span className="font-medium">Currency</span>
          <select name="currency" aria-label="Invoice currency" className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2">
            <option value="USD">US dollar</option>
            <option value="EUR">Euro</option>
            <option value="GBP">British pound</option>
          </select>
        </label>
      </div>
      <label className="block text-sm">
        <span className="font-medium">Notes</span>
        <textarea name="notes" rows={3} placeholder="Thank you for your business!" className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2" />
      </label>
      {error && <p role="alert" className="text-sm text-rose-600">{error}</p>}
      <div className="flex items-center justify-between">
        <button type="button" onClick={() => setLines(lines + 1)} className="text-sm font-medium text-brand-600">
          + Add line item
        </button>
        <div className="flex gap-2">
          <button type="button" onClick={onClose} className="rounded-md px-3 py-2 text-sm text-slate-600">Cancel</button>
          <button type="submit" className="w-24 rounded-md bg-brand-600 py-2 text-sm font-medium text-white">Save</button>
        </div>
      </div>
    </form>
  );
}
