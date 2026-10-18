// Server Components: drop-in for next-intl's `getTranslations` whose `t()` returns `<T>` elements instead of
// strings. The server still renders the shipped text (SEO, no-JS); in the browser `<T>` swaps it for the
// on-device translation once `GlotFallbackProvider` has one.
// ponytail: elements only — attributes (alt, placeholder, <title>/metadata) and t.rich stay server text.
import { createElement, type ReactElement } from "react";
import { getTranslations } from "next-intl/server";
import { T } from "./next-intl.js";

type Values = Record<string, string | number | Date>;

export async function getGlotTranslations(namespace?: string): Promise<(key: string, values?: Values) => ReactElement> {
  const t = (await getTranslations(namespace as never)) as unknown as (key: string, values?: Values) => string;
  return (key, values) => createElement(T, { k: namespace ? `${namespace}.${key}` : key, values }, t(key, values));
}
