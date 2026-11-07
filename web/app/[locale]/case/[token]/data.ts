import type { LaunchStory } from "@/lib/api";

const API = process.env.GLOT_API_URL ?? "http://localhost:8790";

/** Shared launch story; null when the link is unknown or revoked. Never cached: revocation is immediate. */
export async function fetchStory(token: string): Promise<LaunchStory | null> {
  const r = await fetch(`${API}/api/public/launches/${encodeURIComponent(token)}`, { cache: "no-store" }).catch(() => null);
  return r?.ok ? ((await r.json()) as LaunchStory) : null;
}

export function languageName(code: string, locale: string): string {
  try {
    return new Intl.DisplayNames([locale], { type: "language" }).of(code) ?? code;
  } catch {
    return code;
  }
}

export function signedPct(n: number, locale: string): string {
  const f = new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 });
  return `${n >= 0 ? "+" : "−"}${f.format(Math.abs(n))}`;
}
