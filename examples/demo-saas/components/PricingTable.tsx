"use client";

import { useState } from "react";
import { PLANS } from "@/lib/data";
import { PlanCard } from "./PlanCard";

export function PricingTable() {
  const [yearly, setYearly] = useState(false);
  return (
    <>
      <div className="flex justify-center">
        <div className="inline-flex rounded-full bg-slate-100 p-1 text-sm">
          <button onClick={() => setYearly(false)} className={`rounded-full px-4 py-1 ${!yearly ? "bg-white shadow" : ""}`}>
            Monthly
          </button>
          <button onClick={() => setYearly(true)} className={`rounded-full px-4 py-1 ${yearly ? "bg-white shadow" : ""}`}>
            Yearly <span className="text-emerald-600">(2 months free)</span>
          </button>
        </div>
      </div>
      <div className="mt-10 grid gap-6 md:grid-cols-3">
        {PLANS.map((p) => (
          <PlanCard key={p.name} plan={p} yearly={yearly} />
        ))}
      </div>
    </>
  );
}
