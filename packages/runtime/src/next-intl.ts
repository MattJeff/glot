"use client";
// next-intl adapter: wrap your tree (inside your NextIntlClientProvider). When the browser's preferred
// language isn't shipped, Client Components re-render with the on-device translation.
import { createElement, useEffect, useState, type ReactNode } from "react";
import { NextIntlClientProvider, useTimeZone } from "next-intl";
import { fallbackWithBadge, wantedLocale, type AutoOptions } from "./index.js";

export interface GlotFallbackProps extends Omit<AutoOptions, "locale" | "onMessages"> {
  /** Locales the app ships (e.g. `routing.locales`). */
  shipped: readonly string[];
  /** Force a locale (default: `navigator.languages[0]` when not shipped). */
  locale?: string;
  children: ReactNode;
}

export function GlotFallbackProvider({ children, shipped, locale, ...o }: GlotFallbackProps) {
  const timeZone = useTimeZone();
  const [state, set] = useState<{ locale: string; messages: any } | null>(null);
  useEffect(() => {
    const target = locale ?? wantedLocale(shipped);
    if (target) fallbackWithBadge({ ...o, locale: target, onMessages: (messages) => set({ locale: target, messages }) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locale]);
  return state ? createElement(NextIntlClientProvider, { ...state, timeZone, children }) : children;
}
