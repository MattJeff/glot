// SEO helpers for marketing pages (server only). hreflang is declared only for locales whose catalog really
// carries the page's namespace: elsewhere next-intl falls back to English, which is a duplicate, not a translation.
import { readFileSync } from "node:fs";
import path from "node:path";
import type { Metadata } from "next";
import { routing } from "@/i18n/routing";
import { GITHUB_URL } from "@/lib/snippets";

export const SITE = "https://glotlang.com";
const OG_LOCALE: Record<string, string> = { en: "en_US", fr: "fr_FR", es: "es_ES", de: "de_DE", ja: "ja_JP", ar: "ar_AR" };

export function translatedLocales(ns: string): string[] {
  return routing.locales.filter((l) => {
    try {
      return ns in JSON.parse(readFileSync(path.join(process.cwd(), "messages", `${l}.json`), "utf8"));
    } catch {
      return false;
    }
  });
}

export const pageUrl = (locale: string, p = "") => `${SITE}${locale === routing.defaultLocale ? "" : `/${locale}`}${p}`;

/** hreflang map (with x-default) or undefined when the page exists in one language only. */
export function languages(ns: string, p = "") {
  const ls = translatedLocales(ns);
  return ls.length > 1 ? { ...Object.fromEntries(ls.map((l) => [l, pageUrl(l, p)])), "x-default": pageUrl(routing.defaultLocale, p) } : undefined;
}

/** canonical (English when this locale isn't translated), hreflang, Open Graph and Twitter for one page. */
export function pageMetadata({ locale, path: p = "", ns, title, description, absolute }: { locale: string; path?: string; ns: string; title: string; description: string; absolute?: boolean }): Metadata {
  const ls = translatedLocales(ns);
  const own = ls.includes(locale) ? locale : routing.defaultLocale;
  const url = pageUrl(own, p);
  const full = absolute ? title : `${title} · glot`;
  return {
    title: absolute ? { absolute: title } : title,
    description,
    alternates: { canonical: url, languages: languages(ns, p) },
    openGraph: {
      type: "website",
      url,
      siteName: "glot",
      title: full,
      description,
      locale: OG_LOCALE[own],
      alternateLocale: ls.filter((l) => l !== own).map((l) => OG_LOCALE[l]),
    },
    twitter: { card: "summary_large_image", title: full, description },
  };
}

export const ORG_LD = {
  "@context": "https://schema.org",
  "@type": "Organization",
  "@id": `${SITE}/#org`,
  name: "glot",
  alternateName: "glotlang",
  url: `${SITE}/`,
  logo: `${SITE}/favicon.ico`,
  sameAs: [GITHUB_URL, "https://www.npmjs.com/package/glot-i18n"],
};
