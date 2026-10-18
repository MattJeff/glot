"use client";

import { useEffect, useState } from "react";
import type { NewXpExperiment } from "@/lib/api";

/** Intentions offered to the generator (free text is allowed too). */
export const INTENTS = ["social_proof", "benefit", "urgency", "simplicity", "clarity"] as const;
export const ATTRS = {
  device: ["mobile", "tablet", "desktop"],
  source: ["search", "social", "email", "paid", "direct", "referral"],
  visitor: ["new", "returning"],
} as const;
export type SegmentAttr = "country" | "locale" | keyof typeof ATTRS;
export type Segment = { id: string; name: string; rules: Partial<Record<SegmentAttr, string[]>> };

/**
 * What the wizard keeps locally. Arms (the texts) live on the server draft (`id`) as soon as step 1 is done;
 * the rest is PATCHed to that draft as you type, and mirrored here so a reload resumes where you were.
 */
export type Draft = {
  id: string | null;
  step: number;
  name: string;
  keys: string[];
  intents: string[];
  facts: string;
  segments: Segment[];
  goal: string;
  revenue: boolean;
  currency: string;
  minN: number;
  persistence: "none" | "session";
};

export const EMPTY: Draft = {
  id: null,
  step: 0,
  name: "",
  keys: [],
  intents: ["benefit", "social_proof"],
  facts: "",
  segments: [],
  goal: "",
  revenue: false,
  currency: "EUR",
  minN: 500,
  persistence: "session",
};
export const MAX_SEGMENTS = 4; // contract MAX_CELLS
export const MAX_VARIANTS = 3; // arms 1..3: more arms = longer tests, rarely worth it on landing traffic
export const MIN_N = 50;

export const uid = () => Math.random().toString(36).slice(2, 9);

/** Wait until the value has been stable for `ms` (validation, estimates, autosave). */
export function useDebounced<T>(value: T, ms = 400) {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setV(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return v;
}

const storageKey = (slug: string) => `glot:xp-draft:${slug}`;
export function loadDraft(slug: string): Draft | null {
  try {
    const raw = localStorage.getItem(storageKey(slug));
    return raw ? { ...EMPTY, ...(JSON.parse(raw) as Partial<Draft>) } : null;
  } catch {
    return null;
  }
}
export function saveDraft(slug: string, d: Draft | null) {
  try {
    if (d) localStorage.setItem(storageKey(slug), JSON.stringify(d));
    else localStorage.removeItem(storageKey(slug));
  } catch {
    // private mode / quota: the server draft still exists, only the resume shortcut is lost
  }
}

export const segmentsToCells = (segments: Segment[], unnamed: string): NonNullable<NewXpExperiment["cells"]> =>
  segments
    .map((s) => ({
      name: s.name.trim() || unnamed,
      rule: { all: Object.entries(s.rules).filter(([, v]) => v?.length).map(([attr, v]) => ({ attr, in: v! })) },
    }))
    .filter((c) => c.rule.all.length > 0);

/** Server fields mirrored from the local draft (everything but texts). */
export const draftPatch = (d: Draft, unnamed: string): NewXpExperiment => {
  const cells = segmentsToCells(d.segments, unnamed);
  return {
    name: d.name.trim() || d.keys[0] || "experiment",
    keys: d.keys,
    cells,
    allocation: cells.length ? "segmented" : "bandit",
    facts: d.facts.split("\n").map((f) => f.trim()).filter(Boolean),
    goal: d.goal.trim() || undefined,
    metric: d.revenue ? "revenue" : "conversion",
    currency: d.revenue ? d.currency.trim().toUpperCase() || null : null,
    min_n: Math.max(MIN_N, d.minN || MIN_N),
  };
};
