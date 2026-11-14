import { defineRouting } from "next-intl/routing";

export const routing = defineRouting({
  // Every language of the glot cloud project "lot" (glot in glot): web/glot.toml target_locales + en.
  locales: ["en", "fr", "es", "de", "ja", "ar", "pt-BR", "it", "zh-CN", "ko", "nl", "ru", "fi", "cs", "uk", "he", "da", "vi", "sv", "hi", "pl", "id", "th", "tr", "en-GB", "es-MX", "pt-PT", "zh-TW", "el", "hu", "ro", "nb"],
  defaultLocale: "en",
  localePrefix: "as-needed",
  // hreflang is declared per page (lib/seo.ts), only where the page is really translated.
  alternateLinks: false,
});

export type Locale = (typeof routing.locales)[number];
export const RTL_LOCALES: readonly string[] = ["ar", "he"];
