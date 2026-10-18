import { STATUS_META, type InvoiceStatus } from "@/lib/data";

export function StatusBadge({ status }: { status: InvoiceStatus }) {
  const meta = STATUS_META[status];
  return (
    <span title={meta.hint} className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${meta.className}`}>
      {meta.label}
    </span>
  );
}
