import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import Script from "next/script";
import { notFound } from "next/navigation";
import { hasLocale, NextIntlClientProvider } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Providers } from "@/components/providers";
import { UtmCarry } from "@/components/utm-carry";
import { routing, RTL_LOCALES } from "@/i18n/routing";
import "../globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] });

const GLOT_CLOUD = "https://app.glotlang.com";
/** Public (browser) key of the project; rotating it in the dashboard means updating it here. */
const GLOT_PUBLIC_KEY = "pk_6543cf0f139f7404248a5faa";

// ponytail: the 6 historic locales are prerendered at build; the other 25 render on first request and are then
// cached (31 locales × every page timed out the image build). Prerender more if their traffic grows.
const PRERENDERED = ["en", "fr", "es", "de", "ja", "ar"];
export function generateStaticParams() {
  return PRERENDERED.map((locale) => ({ locale }));
}

// viewport-fit=cover: full-bleed on notched phones; globals.css pads with env(safe-area-inset-*).
export const viewport: Viewport = { width: "device-width", initialScale: 1, viewportFit: "cover" };

export async function generateMetadata({ params }: LayoutProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: hasLocale(routing.locales, locale) ? locale : "en", namespace: "home.meta" });
  return { metadataBase: new URL("https://glotlang.com"), title: { default: t("title"), template: "%s · glot" }, description: t("description") };
}

export default async function LocaleLayout({ children, params }: LayoutProps<"/[locale]">) {
  const { locale } = await params;
  if (!hasLocale(routing.locales, locale)) notFound();
  setRequestLocale(locale);

  return (
    <html
      lang={locale}
      dir={RTL_LOCALES.includes(locale) ? "rtl" : "ltr"}
      className={`${geistSans.variable} ${geistMono.variable} h-full antialiased`}
      suppressHydrationWarning
    >
      <body className="min-h-full">
        <NextIntlClientProvider>
          <Providers>{children}</Providers>
          <UtmCarry app={GLOT_CLOUD} />
        </NextIntlClientProvider>
        {/* glot in glot (cloud project "lot"). Insights: cookieless daily counters per language, the signup goal
            feeds Autopilot. Review: ~1 kB loader, inert unless the tab holds a review link. */}
        <Script src="https://unpkg.com/@glot-i18n/insights@0.4.0/dist/insights.global.js" data-key={GLOT_PUBLIC_KEY} data-endpoint={GLOT_CLOUD} data-locale={locale} strategy="afterInteractive" />
        <Script src="/widget/review.global.js" data-project="lot" data-endpoint={GLOT_CLOUD} data-app-origin={GLOT_CLOUD} data-locale={locale} strategy="afterInteractive" />
      </body>
    </html>
  );
}
