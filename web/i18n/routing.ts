import { defineRouting } from "next-intl/routing";

export const routing = defineRouting({
  locales: ["en", "fr", "es", "de", "ja", "ar"],
  defaultLocale: "en",
  localePrefix: "as-needed",
  // hreflang is declared per page (lib/seo.ts), only where the page is really translated.
  alternateLinks: false,
});

export type Locale = (typeof routing.locales)[number];
export const RTL_LOCALES: readonly string[] = ["ar"];
