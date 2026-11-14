"use client";

import { useEffect } from "react";

const KEY = "glot-utm";
const UTM = ["utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content"];

/** utm_* of the current visit (URL first, then what this tab saw earlier). Session storage only: no cookie. */
export function readUtm(): Record<string, string> {
  const fromUrl = Object.fromEntries(UTM.flatMap((k) => (new URLSearchParams(location.search).get(k) ? [[k, new URLSearchParams(location.search).get(k)!]] : [])));
  try {
    if (Object.keys(fromUrl).length) sessionStorage.setItem(KEY, JSON.stringify(fromUrl));
    return Object.keys(fromUrl).length ? fromUrl : JSON.parse(sessionStorage.getItem(KEY) ?? "{}");
  } catch {
    return fromUrl;
  }
}

/** Keeps the ad's utm_* on links to the dashboard (another origin: its storage can't see this one's). */
export function UtmCarry({ app }: { app: string }) {
  useEffect(() => {
    const utm = readUtm();
    if (!Object.keys(utm).length) return;
    const onClick = (e: MouseEvent) => {
      const a = (e.target as HTMLElement).closest?.("a");
      if (!a?.href.startsWith(app)) return;
      const u = new URL(a.href);
      for (const [k, v] of Object.entries(utm)) if (!u.searchParams.has(k)) u.searchParams.set(k, v);
      a.href = u.href;
    };
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, [app]);
  return null;
}
