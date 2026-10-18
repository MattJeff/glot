// Copy-experiment helpers shared by the React adapters (no imports: testable under plain node).

const SK = "glot:insights";

export function readVariants(): Record<string, string> {
  try {
    const v: Record<string, { value: string }> = JSON.parse(sessionStorage.getItem(SK) || "null")?.v ?? {};
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, x.value]));
  } catch {
    return {};
  }
}

/** True once @glot/insights has this session's assignment (legacy client-side draw). */
export function hasVariantState(): boolean {
  try {
    return sessionStorage.getItem(SK) !== null;
  } catch {
    return true; // no storage: nothing will ever arrive, don't mask
  }
}

/** Keeps the edge's segmented-experiment assignments (contract §7) where `@glot/insights` `track()` reads them. */
export function storeAssignments(a: { id: string; cell: string; arm: number }[]): void {
  try {
    const prev: typeof a = JSON.parse(sessionStorage.getItem("glot_xp") || "[]");
    const ids = new Set(a.map((x) => x.id));
    sessionStorage.setItem("glot_xp", JSON.stringify([...prev.filter((x) => !ids.has(x.id)), ...a]));
  } catch {}
}

/** `messages` with each dotted key replaced by its variant; only keys that already exist as strings are touched. */
export function withVariants<M extends Record<string, unknown>>(messages: M, variants: Record<string, string>): M {
  let out = messages;
  for (const [key, value] of Object.entries(variants)) {
    const path = key.split(".");
    const set = (node: unknown, i: number): unknown => {
      if (!node || typeof node !== "object") return node;
      const cur = (node as Record<string, unknown>)[path[i]];
      if (i === path.length - 1) return typeof cur === "string" ? { ...node, [path[i]]: value } : node;
      const next = set(cur, i + 1);
      return next === cur ? node : { ...node, [path[i]]: next };
    };
    out = set(out, 0) as M;
  }
  return out;
}

