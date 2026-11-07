"use client";

import { useEffect, useState } from "react";
import type { NewXpExperiment, XpExperiment, XpSegmentRule } from "@/lib/api";

/** Intentions offered to the generator (free text is allowed too). */
export const INTENTS = ["social_proof", "benefit", "urgency", "simplicity", "clarity"] as const;
export const ATTRS = {
  device: ["mobile", "tablet", "desktop"],
  source: ["search", "social", "email", "paid", "direct", "referral"],
  visitor: ["new", "returning"],
} as const;
export type SegmentAttr = "country" | "locale" | keyof typeof ATTRS;
/** `ref` = server segment id, sent as-is until the segment is edited here (keeps rules the wizard can't show). */
export type Segment = { id: string; name: string; rules: Partial<Record<SegmentAttr, string[]>>; ref?: string };

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
  /** null = every project locale. */
  locales: string[] | null;
  trafficPct: number;
  /** Bandit (more traffic to what wins) vs a fixed even split. */
  autoRebalance: boolean;
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
  locales: null,
  trafficPct: 100,
  autoRebalance: true,
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

export const segmentsToCells = (segments: Segment[], unnamed: string): { name: string; rule: XpSegmentRule }[] =>
  segments
    .map((s) => ({
      name: s.name.trim() || unnamed,
      rule: { all: Object.entries(s.rules).filter(([, v]) => v?.length).map(([attr, v]) => ({ attr, in: v! })) },
    }))
    .filter((c) => c.rule.all.length > 0);

/** Server fields mirrored from the local draft (everything but texts). */
export const draftPatch = (d: Draft, unnamed: string): NewXpExperiment => {
  // Untouched server segments go by id, edited / new ones as {name, rule} (reused by name server-side).
  const cells = d.segments.flatMap((s): NonNullable<NewXpExperiment["cells"]> => (s.ref ? [s.ref] : segmentsToCells([s], unnamed)));
  return {
    name: d.name.trim() || d.keys[0] || "experiment",
    keys: d.keys,
    cells,
    allocation: !d.autoRebalance ? "fixed" : cells.length ? "segmented" : "bandit",
    traffic_pct: Math.min(100, Math.max(1, Math.round(d.trafficPct) || 100)),
    ...(d.locales?.length ? { locales: d.locales } : {}),
    facts: d.facts.split("\n").map((f) => f.trim()).filter(Boolean),
    goal: d.goal.trim() || undefined,
    metric: d.revenue ? "revenue" : "conversion",
    currency: d.revenue ? d.currency.trim().toUpperCase() || null : null,
    min_n: Math.max(MIN_N, d.minN || MIN_N),
  };
};

const EDITABLE = new Set<string>(["country", "locale", "device", "source", "visitor"]);

/** A server draft -> the wizard's state (edit mode). Segments stay referenced by id until edited. */
export function fromExperiment(x: XpExperiment, persistence: Draft["persistence"]): Draft {
  return {
    ...EMPTY,
    id: x.id,
    name: x.name,
    keys: x.keys.map((k) => k.key),
    facts: x.facts.join("\n"),
    segments: x.cells.map((c) => {
      const rules: Segment["rules"] = {};
      for (const p of c.rule.all) {
        const values = p.in ?? (p.eq != null ? [p.eq] : null);
        if (values && EDITABLE.has(p.attr)) rules[p.attr as SegmentAttr] = values;
      }
      // ponytail: not_in / custom.* predicates aren't shown; they survive until this segment is edited here.
      return { id: c.id, name: c.name, rules, ref: c.id };
    }),
    goal: x.goal,
    revenue: x.metric === "revenue",
    currency: x.currency ?? "EUR",
    minN: x.min_n,
    persistence,
    locales: x.locales,
    trafficPct: x.traffic_pct,
    autoRebalance: x.allocation !== "fixed",
  };
}
