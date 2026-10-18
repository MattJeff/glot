"use client";
// @glot/insights: counts the visit; track() credits conversions to the arms the edge assigned (cookie / sessionStorage).
import { useEffect } from "react";
import { init } from "@glot/insights";

export function Beacon({ locale, glotKey, endpoint }: { locale: string; glotKey?: string; endpoint?: string }) {
  useEffect(() => {
    if (glotKey) void init({ key: glotKey, endpoint, locale });
  }, [locale, glotKey, endpoint]);
  return null;
}
