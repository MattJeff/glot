import de from "../messages/de.json";
import en from "../messages/en.json";
import fr from "../messages/fr.json";

export const LOCALES = ["en", "fr", "de"] as const;
export type Locale = (typeof LOCALES)[number];
const CATALOGS: Record<Locale, Record<string, string>> = { en, fr, de };

// ponytail: flat JSON + lookup, no i18n library — the demo is about review, not about formatting.
export const isLocale = (l: string): l is Locale => (LOCALES as readonly string[]).includes(l);
export const translator = (locale: Locale) => (key: keyof typeof en) => CATALOGS[locale][key] ?? en[key];
