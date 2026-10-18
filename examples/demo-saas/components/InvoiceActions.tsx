"use client";

import { useState } from "react";
import { useToast } from "./Toast";
import { ConfirmDialog } from "./ConfirmDialog";

export function InvoiceActions({ number, customer }: { number: string; customer: string }) {
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);

  return (
    <div className="flex gap-2">
      {/* Trap: fixed-width buttons, fine in English, will overflow in German. */}
      <button
        onClick={() => toast("Reminder sent to " + customer)}
        className="w-28 truncate rounded-md border border-slate-200 bg-white py-2 text-sm font-medium hover:bg-slate-50"
      >
        Send reminder
      </button>
      <button
        onClick={() => toast(`Invoice ${number} marked as paid`)}
        className="w-28 rounded-md bg-emerald-600 py-2 text-sm font-medium text-white hover:bg-emerald-700"
      >
        Mark as paid
      </button>
      <button
        onClick={() => setConfirming(true)}
        className="w-20 rounded-md border border-slate-200 bg-white py-2 text-sm text-slate-600 hover:bg-slate-50"
      >
        Archive
      </button>
      {confirming && (
        <ConfirmDialog
          title={"Archive " + number + "?"}
          body="Archived invoices are hidden from your dashboard but stay available in exports."
          confirmLabel="Archive"
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            toast("Invoice archived", "error");
          }}
        />
      )}
    </div>
  );
}
