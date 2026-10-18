// Locale suggestions for the create-project flow. Codes only: names come from Intl.DisplayNames in the UI language.

/** Most requested app languages first; the first 12 are shown before any search. */
export const POPULAR = [
  "fr", "de", "es", "ja", "pt-BR", "it", "zh-CN", "ko", "nl", "ar", "ru", "pl",
  "tr", "sv", "hi", "id", "vi", "uk", "he", "th", "cs", "da", "fi", "nb",
  "ro", "hu", "el", "zh-TW", "pt-PT", "es-MX", "en", "en-GB",
];

// Flags are decorative (aria-hidden) and always shown next to the language name: a language is not a country.
const REGION: Record<string, string> = {
  en: "US", fr: "FR", de: "DE", es: "ES", ja: "JP", it: "IT", ko: "KR", nl: "NL", ar: "SA", ru: "RU",
  pl: "PL", tr: "TR", sv: "SE", hi: "IN", id: "ID", vi: "VN", uk: "UA", he: "IL", th: "TH", cs: "CZ",
  da: "DK", fi: "FI", nb: "NO", ro: "RO", hu: "HU", el: "GR", zh: "CN", pt: "PT",
};

export function flag(code: string) {
  const region = code.split("-").find((p, i) => i > 0 && /^[A-Za-z]{2}$/.test(p)) ?? REGION[code.split("-")[0]];
  if (!region) return "";
  return String.fromCodePoint(...[...region.toUpperCase()].map((c) => 0x1f1a5 + c.charCodeAt(0)));
}

/** A canonical BCP 47 tag, or null ("pt-br" → "pt-BR"). */
export function canonical(tag: string): string | null {
  try {
    const [c] = Intl.getCanonicalLocales(tag.trim());
    return c && /^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$/.test(c) ? c : null;
  } catch {
    return null;
  }
}

/** The browser language, as one of POPULAR when possible ("fr-CA" → "fr"). */
export function browserLocale(): string {
  const tag = canonical(navigator.language ?? "en") ?? "en";
  return POPULAR.includes(tag) ? tag : tag.split("-")[0];
}
