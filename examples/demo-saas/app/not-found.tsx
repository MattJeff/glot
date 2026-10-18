import Link from "next/link";

export default function NotFound() {
  return (
    <div className="py-24 text-center">
      <p className="text-sm font-semibold text-brand-600">404</p>
      <h1 className="mt-2 text-3xl font-bold">Page not found</h1>
      <p className="mt-2 text-slate-500">Sorry, we couldn&apos;t find the page you&apos;re looking for.</p>
      <Link href="/" className="mt-6 inline-block text-sm font-medium text-brand-600">Go back home</Link>
    </div>
  );
}
