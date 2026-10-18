"use client";
// next-intl adapter: wrap your tree (inside your NextIntlClientProvider). When the browser's preferred
// language isn't shipped, Client Components re-render with the on-device translation, and so does every
// Server Component string rendered through `getGlotTranslations` (`@glot/runtime/next-intl/server`) or `<T>`.
import { createContext, createElement, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { NextIntlClientProvider, createTranslator, useTimeZone } from "next-intl";
import { fallbackWithBadge, wantedLocale, type AutoOptions } from "./index.js";

export interface GlotFallbackProps extends Omit<AutoOptions, "locale" | "onMessages"> {
  /** Locales the app ships (e.g. `routing.locales`). */
  shipped: readonly string[];
  /** Force a locale (default: `navigator.languages[0]` when not shipped). */
  locale?: string;
  children: ReactNode;
}

type Values = Record<string, string | number | Date>;
const MISS = "\u0000glot-miss";
const Ctx = createContext<((key: string, values?: Values) => string | null) | null>(null);

export function GlotFallbackProvider({ children, shipped, locale, ...o }: GlotFallbackProps) {
  const timeZone = useTimeZone();
  const [state, set] = useState<{ locale: string; messages: any } | null>(null);
  useEffect(() => {
    const target = locale ?? wantedLocale(shipped);
    if (target) fallbackWithBadge({ ...o, locale: target, onMessages: (messages) => set({ locale: target, messages }) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locale]);
  const t = useMemo(() => {
    if (!state) return null;
    const tr = createTranslator({ ...state, timeZone, onError() {}, getMessageFallback: () => MISS });
    return (key: string, values?: Values) => {
      const s = tr(key as never, values as never) as string;
      return s === MISS ? null : s;
    };
  }, [state, timeZone]);
  return state
    ? createElement(Ctx.Provider, { value: t }, createElement(NextIntlClientProvider, { ...state, timeZone, children }))
    : children;
}

/**
 * Server-rendered text that follows the on-device translation. `children` is what the server rendered
 * (shown as-is until a fallback catalog exists, so hydration always matches); `k` is the full dotted key.
 */
export function T({ k, values, children }: { k: string; values?: Values; children?: ReactNode }) {
  return useContext(Ctx)?.(k, values) ?? children;
}
