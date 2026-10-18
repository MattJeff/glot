export function SearchBar() {
  return (
    <form role="search" className="w-80">
      <label htmlFor="global-search" className="sr-only">
        Search
      </label>
      <input
        id="global-search"
        type="search"
        placeholder="Search invoices, customers…"
        title="Press / to search"
        className="w-full rounded-md border border-slate-200 bg-slate-50 px-3 py-1.5 text-sm outline-none focus:border-brand-500"
      />
    </form>
  );
}
