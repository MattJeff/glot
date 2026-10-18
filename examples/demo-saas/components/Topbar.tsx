import { SearchBar } from "./SearchBar";
import { UserMenu } from "./UserMenu";

export function Topbar() {
  return (
    <header className="flex h-16 items-center justify-between border-b border-slate-200 bg-white px-8">
      <SearchBar />
      <div className="flex items-center gap-4">
        <button aria-label="Notifications" className="relative text-slate-500 hover:text-slate-900">
          🔔
          <span className="absolute -right-1 -top-1 h-2 w-2 rounded-full bg-rose-500" />
        </button>
        <UserMenu />
      </div>
    </header>
  );
}
