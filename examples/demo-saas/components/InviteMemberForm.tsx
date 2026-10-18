"use client";

import { useState } from "react";
import { useToast } from "./Toast";

export function InviteMemberForm({ seatsLeft }: { seatsLeft: number }) {
  const toast = useToast();
  const [email, setEmail] = useState("");
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        toast(`Invitation sent to ${email}`);
        setEmail("");
      }}
      className="flex items-end gap-3"
    >
      <label className="flex-1 text-sm">
        <span className="font-medium">Invite a teammate</span>
        <input
          type="email"
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="name@company.com"
          aria-label="Teammate email address"
          className="mt-1 w-full rounded-md border border-slate-200 px-3 py-2"
        />
      </label>
      <select aria-label="Role" className="rounded-md border border-slate-200 px-3 py-2 text-sm">
        <option>Member</option>
        <option>Admin</option>
      </select>
      <button className="w-28 rounded-md bg-brand-600 py-2 text-sm font-medium text-white">Send invite</button>
      <p className="sr-only">{seatsLeft === 1 ? "1 seat left" : seatsLeft + " seats left"}</p>
    </form>
  );
}
