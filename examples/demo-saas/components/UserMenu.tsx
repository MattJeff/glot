"use client";

import { useState } from "react";

export function UserMenu() {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        onClick={() => setOpen(!open)}
        aria-label="Open account menu"
        aria-expanded={open}
        className="grid h-8 w-8 place-items-center rounded-full bg-slate-900 text-xs font-medium text-white"
      >
        AL
      </button>
      {open && (
        <div className="absolute right-0 z-10 mt-2 w-48 rounded-md border border-slate-200 bg-white py-1 text-sm shadow-lg">
          <p className="px-3 py-2 text-slate-500">Signed in as ada@acme.dev</p>
          <a href="/settings/team" className="block px-3 py-2 hover:bg-slate-50">Profile</a>
          <a href="/settings/billing" className="block px-3 py-2 hover:bg-slate-50">Billing</a>
          <button className="block w-full px-3 py-2 text-left text-rose-600 hover:bg-slate-50">Sign out</button>
        </div>
      )}
    </div>
  );
}
