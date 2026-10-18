import type { Metadata } from "next";
import { invoices } from "@/lib/data";
import { InvoicesView } from "@/components/InvoicesView";

export const metadata: Metadata = { title: "Invoices — Acme Invoices" };

export default function InvoicesPage() {
  return <InvoicesView invoices={invoices} />;
}
