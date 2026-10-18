import Link from "next/link";

export function EmptyState() {
  return (
    <div className="rounded-xl border-2 border-dashed border-slate-200 bg-white p-12 text-center">
      <div className="mx-auto mb-4 grid h-12 w-12 place-items-center rounded-full bg-brand-50 text-xl">✉</div>
      <h3 className="text-lg font-semibold">No invoices yet</h3>
      <p className="mx-auto mt-2 max-w-sm text-sm text-slate-500">
        Create your first invoice and we&apos;ll remind your customer automatically when it&apos;s due.
      </p>
      <Link href="/invoices?new=1" className="mt-6 inline-block w-36 rounded-md bg-brand-600 py-2 text-sm font-medium text-white">
        Create invoice
      </Link>
    </div>
  );
}
