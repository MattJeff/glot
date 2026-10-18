"use client";

import { useState } from "react";
import { ONBOARDING_STEPS } from "@/lib/data";

export function OnboardingChecklist() {
  const [done, setDone] = useState<number[]>([]);
  const remaining = ONBOARDING_STEPS.length - done.length;
  return (
    <div className="rounded-xl border border-slate-200 bg-white p-6">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">Get started</h2>
        <span className="text-sm text-slate-500">
          {remaining === 0 ? "All done! 🎉" : `${remaining} steps left`}
        </span>
      </div>
      <ol className="mt-4 space-y-3">
        {ONBOARDING_STEPS.map((step, i) => (
          <li key={step.title} className="flex items-start gap-3">
            <input
              type="checkbox"
              checked={done.includes(i)}
              onChange={() => setDone(done.includes(i) ? done.filter((d) => d !== i) : [...done, i])}
              aria-label={"Mark step " + (i + 1) + " as done"}
              className="mt-1"
            />
            <div>
              <p className="text-sm font-medium">{step.title}</p>
              <p className="text-sm text-slate-500">{step.description}</p>
            </div>
          </li>
        ))}
      </ol>
      <button className="mt-6 text-sm text-slate-500 hover:text-slate-900">Skip for now</button>
    </div>
  );
}
